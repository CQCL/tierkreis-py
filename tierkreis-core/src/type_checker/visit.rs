use crate::portgraph::graph::NodeIndex;

use super::extract::Extract;
use super::{
    ConstraintSet, Context, GraphWithInputs, InternalizeError, Signature, TypeError, TypeId,
};
use crate::graph;
use crate::graph::{Graph, GraphBuilder, Kind, Node, Value};
use crate::symbol::{FunctionName, Label, TypeVar};
use crate::type_checker::{Constraint, GraphType, Location, Type};
use std::collections::{BTreeMap, HashMap};

pub(super) struct Visitor<'a> {
    pub constraints: ConstraintSet,
    pub signature: &'a Signature,
    pub errors: Vec<TypeError>,
    pub variables: HashMap<TypeVar, (TypeId, Kind)>,
}

impl Visitor<'_> {
    pub fn visit_graph_with_inputs(
        &mut self,
        gwi: &GraphWithInputs,
    ) -> (TypeId, Extract<GraphWithInputs>) {
        // Infer the types of the input values
        let mut input_types = HashMap::new();
        let mut input_extract = HashMap::new();

        for (label, value) in &gwi.inputs {
            let input_context = Context::new(Location::InputValue(*label));
            let (type_id, extract) = self.visit_value(value, &input_context);
            input_types.insert(*label, type_id);
            input_extract.insert(*label, extract);
        }

        // Construct the expected type for the graph from that of the inputs
        let graph_inputs = self.constraints.fresh_closed_row(input_types);
        let graph_outputs = self.constraints.fresh_type(Type::Var(Kind::Row));

        let expected_type = self.constraints.fresh_type(Type::Graph(GraphType {
            inputs: graph_inputs,
            outputs: graph_outputs,
        }));

        // Infer the type of the graph
        let context = Context::empty();
        let (actual_type, graph_extract) = self.visit_graph(&gwi.graph, &context);
        self.constraints
            .add_constraint(Constraint::new_unify(expected_type, actual_type), context);

        let extract = Extract::new(|solved| {
            let inputs = input_extract
                .into_iter()
                .map(|(label, extract)| (label, extract.extract(solved)))
                .collect();
            let graph = graph_extract.extract(solved);
            GraphWithInputs { graph, inputs }
        });

        (actual_type, extract)
    }

    pub fn visit_value(&mut self, value: &Value, context: &Context) -> (TypeId, Extract<Value>) {
        match value {
            Value::Bool(value) => {
                let type_id = self.constraints.fresh_type(Type::Bool);
                let extract = Extract::new_const(Value::Bool(*value));
                (type_id, extract)
            }

            Value::Int(value) => {
                let type_id = self.constraints.fresh_type(Type::Int);
                let extract = Extract::new_const(Value::Int(*value));
                (type_id, extract)
            }

            Value::Str(value) => {
                let type_id = self.constraints.fresh_type(Type::Str);
                let extract = Extract::new_const(Value::Str(value.clone()));
                (type_id, extract)
            }

            Value::Float(value) => {
                let type_id = self.constraints.fresh_type(Type::Float);
                let extract = Extract::new_const(Value::Float(*value));
                (type_id, extract)
            }

            Value::Pair(pair) => {
                let (first_type, first_extract) =
                    self.visit_value(&pair.0, &context.extend(Location::PairFirst));

                let (second_type, second_extract) =
                    self.visit_value(&pair.1, &context.extend(Location::PairSecond));

                let type_id = self
                    .constraints
                    .fresh_type(Type::Pair(first_type, second_type));

                let extract = Extract::new(|solved| {
                    let first = first_extract.extract(solved);
                    let second = second_extract.extract(solved);
                    Value::Pair(Box::new((first, second)))
                });

                (type_id, extract)
            }

            Value::Vec(elements) => {
                let element_var = self.constraints.fresh_type(Type::Var(Kind::Star));
                let mut elements_extract = Vec::new();

                for (index, element) in elements.iter().enumerate() {
                    let (element_type, element_extract) =
                        self.visit_value(element, &context.extend(Location::VecIndex(index)));
                    self.constraints.add_constraint(
                        Constraint::new_unify(element_var, element_type),
                        context.clone(),
                    );
                    elements_extract.push(element_extract)
                }

                let type_id = self.constraints.fresh_type(Type::Vector(element_var));

                let extract = Extract::new(|solved| {
                    Value::Vec(
                        elements_extract
                            .into_iter()
                            .map(|element_extract| element_extract.extract(solved))
                            .collect(),
                    )
                });

                (type_id, extract)
            }

            Value::Struct(fields) => {
                let mut fields_type = BTreeMap::new();
                let mut fields_extract = HashMap::new();

                for (field_name, field_value) in fields {
                    let (field_type, field_extract) = self.visit_value(
                        field_value,
                        &context.extend(Location::StructField(*field_name)),
                    );
                    fields_type.insert(*field_name, field_type);
                    fields_extract.insert(*field_name, field_extract);
                }

                let row = self.constraints.fresh_closed_row(fields_type);
                let type_id = self.constraints.fresh_type(Type::Struct(row, None));

                let rebuild = Extract::new(|solved| {
                    Value::Struct(
                        fields_extract
                            .into_iter()
                            .map(|(name, rb)| (name, rb.extract(solved)))
                            .collect(),
                    )
                });

                (type_id, rebuild)
            }

            Value::Map(elements) => {
                let mut elements_extract = Vec::new();

                let key_type_var = self.constraints.fresh_type(Type::Var(Kind::Star));
                let val_type_var = self.constraints.fresh_type(Type::Var(Kind::Star));

                let key_context = context.extend(Location::MapKey);
                let val_context = context.extend(Location::MapValue);

                for (key, val) in elements {
                    let (key_type, key_extract) = self.visit_value(key, &key_context);
                    self.constraints.add_constraint(
                        Constraint::new_unify(key_type_var, key_type),
                        key_context.clone(),
                    );

                    let (val_type, val_extract) = self.visit_value(val, &val_context);
                    self.constraints.add_constraint(
                        Constraint::new_unify(val_type_var, val_type),
                        val_context.clone(),
                    );

                    elements_extract.push((key_extract, val_extract));
                }

                let type_id = self
                    .constraints
                    .fresh_type(Type::Map(key_type_var, val_type_var));

                let extract = Extract::new(|solved| {
                    let elements = elements_extract
                        .into_iter()
                        .map(|(k, v)| (k.extract(solved), v.extract(solved)))
                        .collect();
                    Value::Map(elements)
                });

                (type_id, extract)
            }

            Value::Graph(graph) => {
                let (graph_type, graph_extract) = self.visit_graph(graph, context);
                let extract = Extract::new(|solved| Value::Graph(graph_extract.extract(solved)));
                (graph_type, extract)
            }

            Value::Variant(tag, value) => {
                let tag = *tag;
                let (value_type, value_extract) = self.visit_value(value, context);

                let (row_type, _) = self.constraints.fresh_open_row([(tag, value_type)]);
                let type_id = self.constraints.fresh_type(Type::Variant(row_type));

                let extract = Extract::new(move |solved| {
                    Value::Variant(tag, Box::new(value_extract.extract(solved)))
                });

                (type_id, extract)
            }
        }
    }

    pub fn visit_graph(&mut self, graph: &Graph, context: &Context) -> (TypeId, Extract<Graph>) {
        let name = graph.name().clone();
        let io_order = [
            graph.inputs().cloned().collect(),
            graph.outputs().cloned().collect(),
        ];
        // Import the graph's edge types into the type checker
        let edge_to_type = {
            let mut edge_to_type = HashMap::new();
            for edge in graph.edges() {
                let edge_type = match &edge.edge_type {
                    Some(edge_type) => self.visit_type(
                        edge_type.clone(),
                        &context.extend(Location::GraphEdge(edge.source, edge.target)),
                    ),
                    None => self.constraints.fresh_type(Type::Var(Kind::Star)),
                };
                edge_to_type.insert((edge.source, edge.target), edge_type);
            }
            edge_to_type
        };

        // Check the types for each node
        let mut nodes_extract: Vec<(NodeIndex, Extract<Node>)> = Vec::new();
        let mut graph_inputs = None;
        let mut graph_outputs = None;
        // track "rest" of node output rows, where unconnected ports will be found
        let mut node_rest_typeids: HashMap<NodeIndex, TypeId> = HashMap::new();
        for node_idx in graph.node_indices() {
            let node = graph.node(node_idx).expect("Node should not be missing.");
            let input_types: BTreeMap<_, _> = graph
                .node_inputs(node_idx)
                .map(|edge| {
                    let port = edge.target.port;
                    let type_ = edge_to_type[&(edge.source, edge.target)];
                    (port, type_)
                })
                .collect();

            let inputs = self.constraints.fresh_closed_row(input_types.clone());

            let output_types: BTreeMap<_, _> = graph
                .node_outputs(node_idx)
                .map(|edge| {
                    let port = edge.source.port;
                    let type_ = edge_to_type[&(edge.source, edge.target)];
                    (port, type_)
                })
                .collect();

            let outputs = if let Node::Input = node {
                // closed row so that we check for inputs precisely
                self.constraints.fresh_closed_row(output_types.clone())
            } else {
                // open row to allow for implicit discarding
                let (outputs, outputs_rest) = self.constraints.fresh_open_row(output_types.clone());
                node_rest_typeids.insert(node_idx, outputs_rest);
                outputs
            };

            let location = match node {
                Node::Input => Location::InputNode,
                Node::Output => Location::OutputNode,
                Node::Const(_) => Location::ConstNode(node_idx),
                Node::Box(_, _) => Location::BoxNode(node_idx),
                Node::Function(function, _) => Location::FunctionNode(node_idx, function.clone()),
                Node::Match => Location::MatchNode(node_idx),
                Node::Tag(_) => Location::TagNode(node_idx),
            };

            let node_context = context.extend(location);

            let actual_type = self
                .constraints
                .fresh_type(Type::Graph(GraphType { inputs, outputs }));

            let expected_type = match node {
                Node::Const(value) => {
                    let (value_type, value_extract) = self.visit_value(value, &node_context);
                    let node_inputs = self.constraints.fresh_type(Type::EmptyRow);
                    let node_outputs = self
                        .constraints
                        .fresh_closed_row(vec![(Label::value(), value_type)]);

                    nodes_extract.push((
                        node_idx,
                        Extract::new(|solved| Node::Const(value_extract.extract(solved))),
                    ));

                    self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: node_inputs,
                        outputs: node_outputs,
                    }))
                }
                Node::Box(loc, graph) => {
                    let (graph_type, graph_rebuild) = self.visit_graph(graph, &node_context);
                    let loc = loc.clone();

                    nodes_extract.push((
                        node_idx,
                        Extract::new(|solved| Node::Box(loc, graph_rebuild.extract(solved))),
                    ));

                    graph_type
                }
                Node::Input => {
                    let node_inputs = self.constraints.fresh_type(Type::EmptyRow);
                    let node_outputs = self.constraints.fresh_type(Type::Var(Kind::Row));
                    graph_inputs = Some(node_outputs);
                    self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: node_inputs,
                        outputs: node_outputs,
                    }))
                }
                Node::Output => {
                    let node_inputs = self.constraints.fresh_type(Type::Var(Kind::Row));
                    let node_outputs = self.constraints.fresh_type(Type::EmptyRow);
                    graph_outputs = Some(node_inputs);
                    self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: node_inputs,
                        outputs: node_outputs,
                    }))
                }
                Node::Function(function, retry_secs) => {
                    nodes_extract.push((
                        node_idx,
                        Extract::new_const(Node::Function(function.clone(), *retry_secs)),
                    ));

                    match self.signature.get(function).cloned() {
                        Some(type_scheme) => self.visit_type_scheme(type_scheme, &node_context),
                        None => {
                            self.errors.push(TypeError::UnknownFunction {
                                function: function.clone(),
                                context: node_context.path(),
                            });
                            self.constraints.fresh_type(Type::Var(Kind::Star))
                        }
                    }
                }
                Node::Match => {
                    nodes_extract.push((node_idx, Extract::new_const(Node::Match)));

                    let mut variant = Vec::new();
                    let mut matchers = Vec::new();

                    let thunk_outputs = self.constraints.fresh_type(Type::Var(Kind::Row));
                    let extra_args = self.constraints.fresh_type(Type::Var(Kind::Row));
                    for label in input_types.keys() {
                        if label == &Label::variant_value() {
                            continue;
                        }

                        let case_type = self.constraints.fresh_type(Type::Var(Kind::Star));

                        let (matcher_input, rest) = self
                            .constraints
                            .fresh_open_row([(Label::value(), case_type)]);
                        self.constraints.add_constraint(
                            Constraint::new_unify(rest, extra_args),
                            node_context.clone(),
                        );

                        let matcher = self.constraints.fresh_type(Type::Graph(GraphType {
                            inputs: matcher_input,
                            outputs: thunk_outputs,
                        }));

                        variant.push((*label, case_type));
                        matchers.push((*label, matcher));
                    }

                    let variant = self.constraints.fresh_closed_row(variant);
                    let variant = self.constraints.fresh_type(Type::Variant(variant));

                    let matchers = self.constraints.fresh_closed_row(matchers);

                    let (expected_inputs, expected_matchers) = self
                        .constraints
                        .fresh_open_row([(Label::variant_value(), variant)]);

                    self.constraints.add_constraint(
                        Constraint::new_unify(expected_matchers, matchers),
                        node_context.clone(),
                    );

                    let output_thunk = self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: extra_args,
                        outputs: thunk_outputs,
                    }));
                    let outputs = self
                        .constraints
                        .fresh_closed_row([(Label::thunk(), output_thunk)]);
                    self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: expected_inputs,
                        outputs,
                    }))
                }
                Node::Tag(tag) => {
                    nodes_extract.push((node_idx, Extract::new_const(Node::Tag(*tag))));
                    let body = self.constraints.fresh_type(Type::Var(Kind::Star));
                    let node_inputs = self.constraints.fresh_closed_row([(Label::value(), body)]);
                    let (row, _) = self.constraints.fresh_open_row([(*tag, body)]);
                    let vt = self.constraints.fresh_type(Type::Variant(row));
                    let outputs = self.constraints.fresh_closed_row([(Label::value(), vt)]);
                    self.constraints.fresh_type(Type::Graph(GraphType {
                        inputs: node_inputs,
                        outputs,
                    }))
                }
            };
            self.constraints.add_constraint(
                Constraint::new_unify(expected_type, actual_type),
                node_context,
            );
        }

        let graph_type = self.constraints.fresh_type(Type::Graph(GraphType {
            inputs: graph_inputs.unwrap(),
            outputs: graph_outputs.unwrap(),
        }));

        let rebuild = Extract::new(move |solved| {
            let mut builder = GraphBuilder::new();
            builder.set_name(name);
            builder.set_io_order(io_order);
            let mut node_ids = Vec::new();

            for (nid, node_extract) in nodes_extract {
                let node = node_extract.extract(solved);
                let added_nid = builder.add_node(node).unwrap();
                debug_assert_eq!(added_nid, nid);
                node_ids.push(nid);
            }

            for node_name in node_ids {
                // find any unconnected ports in the "rest" of the node output
                // and add discard nodes
                if let graph::Type::Row(node_type) =
                    solved.get_type(*node_rest_typeids.get(&node_name).unwrap())
                {
                    for (port, edge_type) in node_type.content {
                        let d = builder.add_node(FunctionName::discard()).unwrap();
                        builder
                            .add_edge((node_name, port), (d, Label::value()), edge_type)
                            .unwrap();
                    }
                }
            }

            for ((source, target), type_id) in edge_to_type {
                let edge_type = solved.get_type(type_id);
                builder.add_edge(source, target, edge_type).unwrap();
            }

            builder.build().unwrap()
        });

        (graph_type, rebuild)
    }

    fn handle_internalize_error(
        &mut self,
        res: Result<TypeId, InternalizeError>,
        type_: impl Into<graph::TypeScheme>,
        context: &Context,
    ) -> TypeId {
        match res {
            Ok(type_id) => type_id,
            Err(error) => {
                self.errors.push(match error {
                    InternalizeError::Kind => TypeError::Kind {
                        type_scheme: type_.into(),
                        context: context.path(),
                    },
                    InternalizeError::UnknownTypeVar(variable) => TypeError::UnknownTypeVar {
                        variable,
                        type_scheme: type_.into(),
                        context: context.path(),
                    },
                });
                self.constraints.fresh_type(Type::Var(Kind::Star))
            }
        }
    }

    pub(crate) fn visit_type(&mut self, type_: graph::Type, context: &Context) -> TypeId {
        let r = self
            .constraints
            .internalize_type(type_.clone(), &mut self.variables);
        self.handle_internalize_error(r, type_, context)
    }

    pub(crate) fn visit_type_rigid(&mut self, type_: graph::Type, context: &Context) -> TypeId {
        let r = self
            .constraints
            .internalize_type_rigid(type_.clone(), &mut self.variables);
        self.handle_internalize_error(r, type_, context)
    }

    fn visit_type_scheme(&mut self, type_scheme: graph::TypeScheme, context: &Context) -> TypeId {
        let r = self
            .constraints
            .internalize_type_scheme(type_scheme.clone(), context);
        self.handle_internalize_error(r, type_scheme, context)
    }
}
