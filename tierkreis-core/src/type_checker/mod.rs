#![allow(missing_docs)]
use self::{extract::Extract, visit::Visitor};
use super::portgraph::graph::NodeIndex;
use crate::graph::{self, Graph};
use crate::graph::{Kind, NodePort, TypeScheme, Value};
use crate::namespace::Namespace;
use crate::symbol::{FunctionName, Label, TypeVar};
use indenter::indented;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::rc::Rc;
use thiserror::Error;

mod extract;
mod solve;
mod union_find;
mod visit;

#[derive(Debug, Clone)]
pub struct GraphWithInputs {
    pub graph: Graph,
    pub inputs: HashMap<Label, Value>,
}

pub type Signature = Namespace<TypeScheme>;

/// Trait for data that can be assigned a type.
pub trait Typeable {
    type Annotated;

    /// Infers a type and fills in the type annotations.
    fn infer_type(
        &self,
        signature: &Signature,
    ) -> Result<(graph::TypeScheme, Self::Annotated), TypeErrors>;
}

impl Typeable for GraphWithInputs {
    type Annotated = Self;

    fn infer_type(
        &self,
        signature: &Signature,
    ) -> std::result::Result<(TypeScheme, Self::Annotated), TypeErrors> {
        infer_type_impl(signature, |visitor| visitor.visit_graph_with_inputs(self))
    }
}

impl Typeable for Value {
    type Annotated = Self;

    fn infer_type(
        &self,
        signature: &Signature,
    ) -> std::result::Result<(TypeScheme, Value), TypeErrors> {
        infer_type_impl(signature, |visitor| {
            visitor.visit_value(self, &Context::empty())
        })
    }
}

impl Typeable for Graph {
    type Annotated = Self;

    fn infer_type(
        &self,
        signature: &Signature,
    ) -> Result<(TypeScheme, Self::Annotated), TypeErrors> {
        infer_type_impl(signature, |visitor| {
            visitor.visit_graph(self, &Context::empty())
        })
    }
}

pub fn value_as_type(
    value: &Value,
    edge_type: graph::Type,
    signature: &Signature,
) -> Result<(TypeScheme, Value), TypeErrors> {
    infer_type_impl(signature, |visitor| {
        let context = Context::empty();
        let (actual_type, type_extract) = visitor.visit_value(value, &context);
        let edge_type_id = visitor.visit_type_rigid(edge_type, &context);
        visitor
            .constraints
            .add_constraint(Constraint::new_unify(edge_type_id, actual_type), context);
        (actual_type, type_extract)
    })
}

fn infer_type_impl<T>(
    signature: &Signature,
    visit: impl FnOnce(&mut Visitor) -> (TypeId, Extract<T>),
) -> Result<(TypeScheme, T), TypeErrors> {
    let mut visitor = Visitor {
        constraints: ConstraintSet::new(),
        signature,
        errors: Vec::new(),
        variables: HashMap::new(),
    };

    let (type_id, extract) = visit(&mut visitor);

    let solution = match solve::solve(&visitor.constraints) {
        Ok(solution) if visitor.errors.is_empty() => solution,
        Ok(_) => return Err(TypeErrors(visitor.errors)),
        Err(mut type_errors) => {
            type_errors.0.extend(visitor.errors);
            return Err(type_errors);
        }
    };

    let value = extract.extract(&solution);

    let type_scheme = TypeScheme {
        variables: solution.variables().collect(),
        constraints: solution.bounds().collect(),
        body: solution.get_type(type_id),
    };

    Ok((type_scheme, value))
}

/// Collection of [`TypeError`]s.
#[derive(Debug, Clone, Error)]
pub struct TypeErrors(pub Vec<TypeError>);

impl TypeErrors {
    pub fn errors(&self) -> impl Iterator<Item = &TypeError> {
        self.0.iter()
    }
}

impl std::fmt::Display for TypeErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for error in self.0.iter() {
            writeln!(f, "{}\n", error)?;
        }

        Ok(())
    }
}

/// Errors that can occur during type checking.
///
/// Each error is annotated with a path of [`Location`]s where it originated.
#[derive(Debug, Clone, Error)]
pub enum TypeError {
    Unify {
        expected: graph::Type,
        found: graph::Type,
        context: Vec<Location>,
    },
    Kind {
        type_scheme: graph::TypeScheme,
        context: Vec<Location>,
    },
    UnknownFunction {
        function: FunctionName,
        context: Vec<Location>,
    },
    UnknownTypeVar {
        variable: TypeVar,
        type_scheme: graph::TypeScheme,
        context: Vec<Location>,
    },
    Bound {
        bound: graph::Constraint,
        context: Vec<Location>,
    },
}

impl TypeError {
    pub fn context(&self) -> &[Location] {
        match self {
            TypeError::Unify { context, .. } => context,
            TypeError::Kind { context, .. } => context,
            TypeError::UnknownFunction { context, .. } => context,
            TypeError::UnknownTypeVar { context, .. } => context,
            TypeError::Bound { context, .. } => context,
        }
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let indent = "    ";

        match self {
            TypeError::Unify {
                expected, found, ..
            } => {
                writeln!(f, "Type mismatch.\n")?;
                writeln!(f, "Failed to unify types\n")?;
                writeln!(indented(f).with_str(indent), "{:#?}\n", expected)?;
                writeln!(f, "and\n")?;
                writeln!(indented(f).with_str(indent), "{:#?}", found)?;
            }
            TypeError::Kind { type_scheme, .. } => {
                writeln!(f, "Kind error.\n")?;
                writeln!(f, "Encountered ill-kinded type\n")?;
                writeln!(indented(f).with_str(indent), "{:#?}\n", type_scheme)?;
            }
            TypeError::UnknownFunction { function, .. } => {
                writeln!(f, "Unknown function.\n")?;
                writeln!(f, "Encountered node with unknown function '{}'.", function)?;
            }
            TypeError::UnknownTypeVar {
                variable,
                type_scheme,
                ..
            } => {
                writeln!(f, "Missing type variable.\n")?;
                writeln!(
                    f,
                    "Encountered unknown type variable '{}' in type scheme\n",
                    variable
                )?;
                writeln!(indented(f).with_str(indent), "{:#?}\n", type_scheme)?;
                writeln!(
                    f,
                    "Hint: Every type variable must be mentioned in the type scheme."
                )?;
            }
            TypeError::Bound { .. } => {
                writeln!(f, "Unsolveable type bound.")?;
                // TODO: More information!
            }
        }

        Ok(())
    }
}

/// Reference to a node in the type checker's representation of types. By using [TypeId]s instead
/// of recursive references, the type checker can quickly unify and compare subterms of a type and
/// represent types as a directed acyclic graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TypeId(usize);

/// Internal type data structure that mirrors [crate::graph::Type] but uses [TypeId]s.
#[derive(Debug, Clone)]
enum Type {
    Int,
    Bool,
    Str,
    Float,
    Vector(TypeId),
    Pair(TypeId, TypeId),
    Graph(GraphType),
    Var(Kind),
    RigidVar(TypeVar), // Original TypeVar treated as a concrete type
    Map(TypeId, TypeId),
    Struct(TypeId, Option<String>),
    Row(RowType),
    EmptyRow,
    Variant(TypeId),
}

impl Type {
    fn children(&self) -> Vec<TypeId> {
        match self {
            Type::Int | Type::Bool | Type::Str | Type::Float => vec![],
            Type::Var(_) | Type::RigidVar(_) => vec![],
            Type::Vector(element) => vec![*element],
            Type::Pair(first, second) => vec![*first, *second],
            Type::Map(first, second) => vec![*first, *second],
            Type::Graph(graph) => vec![graph.inputs, graph.outputs],
            Type::Row(row) => {
                let mut children = Vec::new();
                children.extend(row.content.values().copied());
                children.push(row.rest);
                children
            }
            Type::Struct(row, _) => vec![*row],
            Type::EmptyRow => vec![],
            Type::Variant(row) => vec![*row],
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Type::Var(kind) => *kind,
            Type::Row(_) => Kind::Row,
            Type::EmptyRow => Kind::Row,
            _ => Kind::Star,
        }
    }
}

/// Internal type data structure that mirrors [crate::graph::GraphType] but uses [TypeId]s.
#[derive(Debug, Clone)]
struct GraphType {
    inputs: TypeId,
    outputs: TypeId,
}

/// Internal type data structure that mirrors [crate::graph::RowType] but uses [TypeId]s.
#[derive(Debug, Clone)]
struct RowType {
    content: BTreeMap<Label, TypeId>,
    rest: TypeId,
}

#[derive(Debug, Clone)]
struct ConstraintSet {
    types: Vec<Type>,
    constraints: Vec<ConstraintData>,
}

impl ConstraintSet {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            constraints: Vec::new(),
        }
    }

    pub fn fresh_type(&mut self, type_: Type) -> TypeId {
        let id = TypeId(self.types.len());
        self.types.push(type_);
        id
    }

    // row from fixed content
    fn fresh_closed_row<I>(&mut self, content: I) -> TypeId
    where
        I: IntoIterator<Item = (Label, TypeId)>,
    {
        let rest = self.fresh_type(Type::EmptyRow);
        self.fresh_type(Type::Row(RowType {
            content: content.into_iter().collect(),
            rest,
        }))
    }

    // row which may have entries in rest, return ID for row and rest seprately
    fn fresh_open_row<I>(&mut self, content: I) -> (TypeId, TypeId)
    where
        I: IntoIterator<Item = (Label, TypeId)>,
    {
        let rest = self.fresh_type(Type::Var(Kind::Row));
        (
            self.fresh_type(Type::Row(RowType {
                content: content.into_iter().collect(),
                rest,
            })),
            rest,
        )
    }

    pub fn internalize_type_scheme(
        &mut self,
        type_scheme: impl Into<TypeScheme>,
        context: &Context,
    ) -> Result<TypeId, InternalizeError> {
        let type_scheme: TypeScheme = type_scheme.into();
        let mut variables = HashMap::new();

        let mut worker = Internalize {
            constraints: self,
            variables: &mut variables,
            rigid: false,
        };

        let type_id = worker.internalize_type(type_scheme.body, Kind::Star)?;

        for constraint in type_scheme.constraints {
            worker.internalize_constraint(constraint, context)?;
        }

        for (var, (_, kind)) in variables {
            match type_scheme.variables.get(&var) {
                Some(expected_kind) => {
                    if *expected_kind != kind {
                        return Err(InternalizeError::Kind);
                    }
                }
                None => {
                    return Err(InternalizeError::UnknownTypeVar(var));
                }
            }
        }

        Ok(type_id)
    }

    pub fn internalize_type(
        &mut self,
        type_: graph::Type,
        variables: &mut HashMap<TypeVar, (TypeId, Kind)>,
    ) -> Result<TypeId, InternalizeError> {
        let mut worker = Internalize {
            constraints: self,
            variables,
            rigid: false,
        };

        worker.internalize_type(type_, Kind::Star)
    }

    /// Internalizes a type, but marks all type variables within it as "rigid", meaning
    /// that they can only be unified with type variables that are *not* rigid
    pub fn internalize_type_rigid(
        &mut self,
        type_: graph::Type,
        variables: &mut HashMap<TypeVar, (TypeId, Kind)>,
    ) -> Result<TypeId, InternalizeError> {
        let mut worker = Internalize {
            constraints: self,
            variables,
            rigid: true,
        };

        worker.internalize_type(type_, Kind::Star)
    }

    pub fn add_constraint(&mut self, constraint: impl Into<Constraint>, context: Context) {
        self.constraints.push(ConstraintData {
            constraint: constraint.into(),
            context,
        });
    }

    pub fn constraint(&self, id: ConstraintId) -> Option<&ConstraintData> {
        self.constraints.get(id.0)
    }
}

impl Default for ConstraintSet {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
enum Constraint {
    Unify { expected: TypeId, found: TypeId },
    Bound(Bound),
}

impl Constraint {
    pub fn new_unify(expected: TypeId, found: TypeId) -> Self {
        Constraint::Unify { expected, found }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Bound {
    Lacks(TypeId, Label),
    Partition(TypeId, TypeId, TypeId),
}

impl Bound {
    pub fn types(&self) -> impl Iterator<Item = TypeId> + '_ {
        match *self {
            Bound::Lacks(row, _) => vec![row].into_iter(),
            Bound::Partition(r, s, t) => vec![r, s, t].into_iter(),
        }
    }
}

impl From<Bound> for Constraint {
    fn from(bound: Bound) -> Self {
        Self::Bound(bound)
    }
}

#[derive(Debug, Clone)]
struct ConstraintData {
    pub constraint: Constraint,
    pub context: Context,
}

#[derive(Debug, Clone)]
struct Context(Option<Rc<ContextData>>);

impl Context {
    pub fn path(&self) -> Vec<Location> {
        match self.0.as_ref() {
            None => vec![],
            Some(rc) => {
                let mut v = rc.parent.path();
                v.push(rc.location.clone());
                v
            }
        }
    }

    pub fn extend(&self, location: Location) -> Self {
        Context(Some(Rc::new(ContextData {
            location,
            parent: self.clone(),
        })))
    }

    pub fn empty() -> Self {
        Self(None)
    }

    pub fn new(location: Location) -> Self {
        Self::empty().extend(location)
    }
}

#[derive(Debug, Clone)]
struct ContextData {
    location: Location,
    parent: Context,
}

impl Default for Context {
    fn default() -> Self {
        Self::empty()
    }
}

/// Hints to localise a type error in a graph or value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    VecIndex(usize),
    FunctionNode(NodeIndex, FunctionName),
    BoxNode(NodeIndex),
    ConstNode(NodeIndex),
    MatchNode(NodeIndex),
    TagNode(NodeIndex),
    GraphEdge(NodePort, NodePort),
    InputNode,
    OutputNode,
    StructField(Label),
    PairFirst,
    PairSecond,
    MapKey,
    MapValue,
    InputValue(Label),
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Location::VecIndex(index) => write!(f, "vec index: {}", index),
            Location::StructField(field) => write!(f, "struct field: {}", field),
            Location::PairFirst => write!(f, "first component of pair"),
            Location::PairSecond => write!(f, "second component of pair"),
            Location::MapKey => write!(f, "map key"),
            Location::MapValue => write!(f, "map value"),
            Location::FunctionNode(node, function) => {
                write!(f, "graph node '{:?}' with function '{}'", node, function)
            }
            Location::BoxNode(node) => write!(f, "box node '{}'", node.index()),
            Location::ConstNode(node) => write!(f, "const node '{}'", node.index()),
            Location::InputNode => write!(f, "input node"),
            Location::OutputNode => write!(f, "output node"),
            Location::GraphEdge(source, target) => {
                write!(f, "edge from '{}' to '{}'", source, target)
            }
            Location::InputValue(label) => write!(f, "input value on port: {}", label),
            Location::MatchNode(node) => write!(f, "match node '{}'", node.index()),
            Location::TagNode(node) => write!(f, "tag node '{}'", node.index()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ConstraintId(usize);

struct Internalize<'a, 'b> {
    constraints: &'a mut ConstraintSet,
    variables: &'b mut HashMap<TypeVar, (TypeId, Kind)>,
    rigid: bool,
}

impl Internalize<'_, '_> {
    fn get_variable(&mut self, var: TypeVar, kind: Kind) -> Result<TypeId, InternalizeError> {
        match self.variables.get(&var) {
            Some((_, actual_kind)) if *actual_kind != kind => Err(InternalizeError::Kind),
            Some((type_id, _)) => Ok(*type_id),
            None => {
                let type_id = self.constraints.fresh_type(if self.rigid {
                    Type::RigidVar(var)
                } else {
                    Type::Var(kind)
                });
                self.variables.insert(var, (type_id, kind));
                Ok(type_id)
            }
        }
    }

    fn internalize_type(
        &mut self,
        type_: graph::Type,
        kind: Kind,
    ) -> Result<TypeId, InternalizeError> {
        let ground = match type_ {
            graph::Type::Bool => Type::Bool,
            graph::Type::Int => Type::Int,
            graph::Type::Str => Type::Str,
            graph::Type::Float => Type::Float,
            graph::Type::Graph(graph) => {
                let inputs = self.internalize_type(graph::Type::Row(graph.inputs), Kind::Row)?;
                let outputs = self.internalize_type(graph::Type::Row(graph.outputs), Kind::Row)?;
                Type::Graph(GraphType { inputs, outputs })
            }
            graph::Type::Pair(first, second) => {
                let first = self.internalize_type(*first, kind)?;
                let second = self.internalize_type(*second, kind)?;
                Type::Pair(first, second)
            }
            graph::Type::Map(first, second) => {
                let first = self.internalize_type(*first, kind)?;
                let second = self.internalize_type(*second, kind)?;
                Type::Map(first, second)
            }
            graph::Type::Vec(element) => Type::Vector(self.internalize_type(*element, kind)?),
            graph::Type::Var(var) => return self.get_variable(var, kind),
            graph::Type::Row(row) => {
                let content: BTreeMap<Label, TypeId> = row
                    .content
                    .into_iter()
                    .map(|(label, typ_)| Ok((label, self.internalize_type(typ_, Kind::Star)?)))
                    .collect::<Result<_, InternalizeError>>()?;
                let rest = match row.rest {
                    Some(var) => self.get_variable(var, Kind::Row)?,
                    None => self.constraints.fresh_type(Type::EmptyRow),
                };
                Type::Row(RowType { content, rest })
            }
            graph::Type::Struct(row, name) => Type::Struct(
                self.internalize_type(graph::Type::Row(row), Kind::Row)?,
                name,
            ),
            graph::Type::Variant(row) => {
                Type::Variant(self.internalize_type(graph::Type::Row(row), Kind::Row)?)
            }
        };

        if ground.kind() != kind {
            return Err(InternalizeError::Kind);
        }

        Ok(self.constraints.fresh_type(ground))
    }

    pub fn internalize_constraint(
        &mut self,
        constraint: graph::Constraint,
        context: &Context,
    ) -> Result<(), InternalizeError> {
        match constraint {
            graph::Constraint::Lacks { row, label } => {
                let row = self.internalize_type(row, Kind::Row)?;
                self.constraints
                    .add_constraint(Bound::Lacks(row, label), context.clone());
                Ok(())
            }
            graph::Constraint::Partition { left, right, union } => {
                let r = self.internalize_type(left, Kind::Row)?;
                let s = self.internalize_type(right, Kind::Row)?;
                let t = self.internalize_type(union, Kind::Row)?;
                self.constraints
                    .add_constraint(Bound::Partition(r, s, t), context.clone());
                Ok(())
            }
        }
    }
}

#[derive(Debug, Error)]
enum InternalizeError {
    #[error("invalid kind")]
    Kind,
    #[error("unknown type variable: {0}")]
    UnknownTypeVar(TypeVar),
}

#[cfg(test)]
mod tests {
    use super::{solve, TypeErrors, Typeable};
    use crate::builtins::{
        builtin_copy, builtin_discard, builtin_eval, builtin_parallel, builtin_switch,
    };
    use crate::graph::{self, Graph, GraphBuilder, Kind, RowType, TypeScheme, Value};
    use crate::graph::{GraphType, Node, Type};
    use crate::namespace::Namespace;
    use crate::prelude::{TryFrom, TryInto};
    use crate::symbol::{Label, Name, TypeVar};
    use crate::type_checker::{Constraint, ConstraintSet, Context, Signature, Visitor};
    use single::Single;
    use std::collections::{BTreeMap, HashMap};
    use std::error::Error;

    fn type_magic() -> TypeScheme {
        let a = TypeVar::symbol("a");

        let mut t = GraphType::new();
        t.add_output(Label::value(), a);
        TypeScheme::from(t).with_variable(a, Kind::Star)
    }

    fn type_repeat() -> TypeScheme {
        let a = TypeVar::symbol("a");
        let count = Label::symbol("count");
        let values = Label::symbol("values");

        let mut t = GraphType::new();
        t.add_input(Label::value(), a);
        t.add_input(count, Type::Int);
        t.add_output(values, graph::Type::Vec(Box::new(a.into())));
        TypeScheme::from(t).with_variable(a, Kind::Star)
    }

    fn signature() -> Namespace<TypeScheme> {
        let mut signature = Namespace::new();
        signature
            .functions
            .insert(Name::symbol("repeat"), type_repeat());
        signature
            .functions
            .insert(Name::symbol("discard"), builtin_discard().type_scheme);
        signature
            .functions
            .insert(Name::symbol("copy"), builtin_copy().type_scheme);
        signature
            .functions
            .insert(Name::symbol("magic"), type_magic());
        signature
            .functions
            .insert(Name::symbol("eval"), builtin_eval().type_scheme);
        signature
            .functions
            .insert(Name::symbol("parallel"), builtin_parallel().type_scheme);
        signature
            .functions
            .insert(Name::symbol("switch"), builtin_switch().type_scheme);
        signature
    }

    #[test]
    fn test_infer() -> Result<(), Box<dyn Error>> {
        let mut builder = GraphBuilder::new();
        builder.set_name("g_name".into());
        builder.set_io_order([
            vec![
                TryInto::try_into("input_count")?,
                TryInto::try_into("input_value")?,
            ],
            vec![],
        ]);
        let [input, _] = Graph::boundary();
        let repeat0 = builder.add_node("repeat")?;
        let repeat1 = builder.add_node("repeat")?;
        let copy = builder.add_node("copy")?;
        let sink = builder.add_node("discard")?;

        builder.add_edge((input, "input_count"), (copy, "value"), None)?;
        builder.add_edge((input, "input_value"), (repeat0, "value"), Type::Bool)?;
        builder.add_edge((copy, "value_0"), (repeat0, "count"), None)?;
        builder.add_edge((copy, "value_1"), (repeat1, "count"), None)?;
        builder.add_edge((repeat0, "values"), (repeat1, "value"), None)?;
        builder.add_edge((repeat1, "values"), (sink, "value"), None)?;

        let graph = builder.build()?;

        let (_, graph) = graph.infer_type(&signature())?;
        assert_eq!(graph.name(), "g_name");
        assert_eq!(
            graph.inputs().collect::<Vec<_>>(),
            vec![
                &TryInto::try_into("input_count")?,
                &TryInto::try_into("input_value")?
            ]
        );
        let array_bool = Type::Vec(Box::new(Type::Bool));
        let array_array_bool = Type::Vec(Box::new(array_bool.clone()));

        let expected_types = &[
            ((input, "input_count"), Type::Int),
            ((input, "input_value"), Type::Bool),
            ((copy, "value_0"), Type::Int),
            ((copy, "value_1"), Type::Int),
            ((repeat0, "values"), array_bool),
            ((repeat1, "values"), array_array_bool),
        ];

        for (output_port, expected_type) in expected_types {
            let actual_type = graph
                .node_output(*output_port)
                .unwrap()
                .edge_type
                .as_ref()
                .unwrap();
            assert_eq!(
                expected_type, actual_type,
                "type at output port {:?}",
                output_port
            );
        }

        Ok(())
    }

    fn auto_variant<const N: usize>(arr: [(&'static str, Type); N]) -> Type {
        use std::iter::FromIterator;
        let names: Vec<String> = Vec::from_iter(arr.iter().map(|(k, _)| (*k).into()));
        Type::Variant(RowType {
            content: BTreeMap::from_iter(arr.map(|(k, v)| (Label::symbol(k), v))),
            rest: Some(TypeVar::symbol(names.join("_"))),
        })
    }

    #[test]
    fn test_variant_union() -> Result<(), Box<dyn Error>> {
        let [input, output] = Graph::boundary();
        let graph = {
            let mut builder = GraphBuilder::new();

            let if_n = builder.add_node("switch")?;
            builder.add_edge(
                (input, "arg1"),
                (if_n, "if_true"),
                auto_variant([("v1", Type::Int), ("v2", Type::Bool)]),
            )?;
            builder.add_edge(
                (input, "arg2"),
                (if_n, "if_false"),
                auto_variant([("v3", Type::Float), ("v2", Type::Bool)]),
            )?;
            builder.add_edge((input, "which"), (if_n, "pred"), Type::Bool)?;
            builder.add_edge((if_n, "value"), (output, "value"), None)?;
            builder.build()?
        };
        let (_, graph) = graph.infer_type(&signature())?;
        match &graph.node_inputs(output).single().edge_type {
            Some(Type::Variant(RowType { content, rest })) => {
                assert!(rest.is_some());
                assert_eq!(
                    *content,
                    BTreeMap::from([
                        (TryInto::try_into("v1")?, Type::Int),
                        (TryInto::try_into("v2")?, Type::Bool),
                        (TryInto::try_into("v3")?, Type::Float)
                    ])
                );
            }
            _ => panic!(),
        };
        Ok(())
    }

    #[test]
    fn test_variant_tag_and_value() -> Result<(), Box<dyn Error>> {
        let v1 = TryInto::try_into("v1")?;
        let v2 = TryInto::try_into("v2")?;
        let [input, output] = Graph::boundary();
        let variant2;
        let graph = {
            let mut builder = GraphBuilder::new();
            let if_n = builder.add_node("switch")?;

            let variant1 = builder.add_node(Value::Variant(v1, Box::new(Value::Int(5))))?;
            let c = builder.add_node(Value::Float(3.1))?;
            variant2 = builder.add_node(Node::Tag(v2))?;
            builder.add_edge((c, "value"), (variant2, "value"), None)?;
            builder.add_edge((input, "which"), (if_n, "pred"), Type::Bool)?;
            builder.add_edge((variant1, "value"), (if_n, "if_true"), None)?;
            builder.add_edge((variant2, "value"), (if_n, "if_false"), None)?;
            builder.add_edge((if_n, "value"), (output, "value"), None)?;
            builder.build()?
        };
        let (_, graph) = graph.infer_type(&signature())?;
        let variant2 = graph.node_outputs(variant2).single();
        match &variant2.edge_type {
            Some(Type::Variant(RowType { content, rest })) => {
                assert!(rest.is_some());
                assert!(content.len() <= 2);
                assert_eq!(content[&v2], Type::Float);
                // Allow typechecker to infer variant type with or without v1:Int
                assert!(matches!(content.get(&v1), None | Some(Type::Int)));
            }
            _ => panic!(),
        };
        let output = graph.node_inputs(output).single();
        match &output.edge_type {
            Some(Type::Variant(RowType { content, rest })) => {
                assert!(rest.is_some());
                assert_eq!(content.len(), 2);
                assert_eq!(content[&v1], Type::Int);
                assert_eq!(content[&v2], Type::Float);
            }
            _ => panic!(),
        };

        Ok(())
    }

    #[test]
    fn test_eval_infer() -> Result<(), Box<dyn Error>> {
        let a: Label = TryInto::try_into("a")?;
        let b: Label = TryInto::try_into("b")?;
        let c: Label = TryInto::try_into("c")?;
        let [input, output] = Graph::boundary();

        let magic;
        let graph = {
            let mut builder = GraphBuilder::new();

            let eval = builder.add_node("eval")?;
            magic = builder.add_node("magic")?;

            builder.add_edge((input, "input_a"), (eval, a), Type::Int)?;
            builder.add_edge((input, "input_b"), (eval, b), Type::Bool)?;
            builder.add_edge((eval, c), (output, "output_c"), Type::Str)?;
            builder.add_edge((magic, "value"), (eval, "thunk"), None)?;

            builder.build()?
        };

        let (_, graph) = graph.infer_type(&signature())?;

        let mut expected_type = GraphType::new();
        expected_type.add_input(a, Type::Int);
        expected_type.add_input(b, Type::Bool);
        expected_type.add_output(c, Type::Str);
        expected_type.outputs.rest = Some(TryInto::try_into("var22")?);
        let expected_type = Type::Graph(expected_type);

        let actual_type = graph
            .node_output((magic, "value"))
            .unwrap()
            .edge_type
            .as_ref()
            .unwrap();

        assert_eq!(&expected_type, actual_type);

        Ok(())
    }

    #[test]
    fn test_eval_const() -> Result<(), Box<dyn Error>> {
        let [input, output] = Graph::boundary();
        let mut builder = GraphBuilder::new();
        let repeat = builder.add_node("repeat")?;
        builder.add_edge((input, "count"), (repeat, "count"), None)?;
        builder.add_edge((input, "value"), (repeat, "value"), None)?;
        builder.add_edge((repeat, "values"), (output, "values"), None)?;
        let nested_graph = builder.build()?;

        let mut builder = GraphBuilder::new();
        let const_ = builder.add_node(Value::Graph(nested_graph))?;
        let eval = builder.add_node("eval")?;
        builder.add_edge((input, "count"), (eval, "count"), None)?;
        builder.add_edge((input, "value"), (eval, "value"), Type::Bool)?;
        builder.add_edge((const_, "value"), (eval, "thunk"), None)?;
        builder.add_edge((eval, "values"), (output, "values"), None)?;
        let graph = builder.build()?;

        let (_, graph) = graph.infer_type(&signature())?;

        let expected_type = Type::Vec(Box::new(Type::Bool));

        // Check that the eval's output node has been annotated correctly. This requires the type
        // of the 'repeat' node to have propagated from the nested graph into the parent graph.
        assert_eq!(
            graph
                .node_output((eval, "values"))
                .unwrap()
                .edge_type
                .as_ref()
                .unwrap(),
            &expected_type
        );

        // Check that the nested graph has been annotated with the correct types. This requires
        // type information from the parent graph to have been propagated into the nested one.
        let nested_graph = match graph.node(const_).unwrap() {
            Node::Const(Value::Graph(nested_graph)) => nested_graph,
            _ => panic!(),
        };

        assert_eq!(
            nested_graph
                .node_output((repeat, "values"))
                .unwrap()
                .edge_type
                .as_ref()
                .unwrap(),
            &expected_type
        );

        Ok(())
    }

    #[test]
    fn test_infer_match() -> Result<(), Box<dyn Error>> {
        use crate::graph::{Graph, GraphBuilderError};

        let mut sig: Signature = signature();
        sig.insert_into_namespace(
            "int_to_str".parse()?,
            Type::Graph(GraphType {
                inputs: RowType {
                    content: BTreeMap::from([(TryInto::try_into("int_arg")?, Type::Int)]),
                    rest: None,
                },
                outputs: RowType {
                    content: BTreeMap::from([(TryInto::try_into("str_res")?, Type::Str)]),
                    rest: None,
                },
            })
            .into(),
        );
        let [input, output] = Graph::boundary();

        let v1func = Value::Graph({
            let mut v1_builder = GraphBuilder::new();
            let i2s = v1_builder.add_node("int_to_str")?;
            v1_builder.add_edge((input, "value"), (i2s, "int_arg"), Type::Int)?;
            v1_builder.add_edge((i2s, "str_res"), (output, "result"), Type::Str)?;
            let d = v1_builder.add_node("discard")?;
            v1_builder.add_edge((input, "xtra"), (d, "value"), None)?;
            v1_builder.build()?
        });
        let v2func = Value::Graph({
            let mut v2_builder = GraphBuilder::new();
            let b2s = v2_builder.add_node("switch")?;
            let f = v2_builder.add_node(Value::Str("FALSE".into()))?;
            v2_builder.add_edge((input, "value"), (b2s, "pred"), Type::Bool)?;
            v2_builder.add_edge((input, "xtra"), (b2s, "if_true"), Type::Str)?;
            v2_builder.add_edge((f, "value"), (b2s, "if_false"), Type::Str)?;
            v2_builder.add_edge((b2s, "value"), (output, "result"), None)?;
            v2_builder.build()?
        });

        fn build_match_graph<const N: usize>(
            arr: [(&'static str, Value); N],
        ) -> Result<Graph, GraphBuilderError> {
            let mut builder = GraphBuilder::new();
            let [input, output] = Graph::boundary();

            let matcher = builder.add_node(Node::Match)?;
            builder.add_edge(
                (input, "var"),
                (matcher, Label::variant_value()),
                Some(auto_variant([("v1", Type::Int), ("v2", Type::Bool)])),
            )?;

            for (port, val) in arr {
                let con = builder.add_node(Node::Const(val))?;
                builder.add_edge((con, "value"), (matcher, port), None)?;
            }
            builder.add_edge((matcher, "thunk"), (output, "res"), None)?;
            builder.build()
        }

        let good_graph = build_match_graph([("v1", v1func.clone()), ("v2", v2func.clone())])?;
        let expected_type = Type::Graph(GraphType {
            inputs: RowType {
                content: BTreeMap::from([(TryInto::try_into("xtra")?, Type::Str)]),
                rest: None,
            },
            outputs: RowType {
                content: BTreeMap::from([(TryInto::try_into("result")?, Type::Str)]),
                rest: None,
            },
        });
        let (_, good_graph) = good_graph.infer_type(&sig)?;
        assert_eq!(
            good_graph.node_inputs(output).single().edge_type,
            Some(expected_type.clone())
        );
        // Extra matchers are considered OK:
        let extra_cases = build_match_graph([
            ("v1", v1func.clone()),
            ("v2", v2func.clone()),
            ("v3", v2func.clone()),
        ])?;
        let (_, extra_cases) = extra_cases.infer_type(&sig)?;
        assert_eq!(
            extra_cases.node_inputs(output).single().edge_type,
            Some(expected_type)
        );

        let missing_case = build_match_graph([("v1", v1func.clone())])?;
        assert!(missing_case.infer_type(&sig).is_err());

        let wrong_case = build_match_graph([("v1", v1func.clone()), ("v2", Value::Int(6))])?;
        assert!(wrong_case.infer_type(&sig).is_err());

        let switched_cases = build_match_graph([("v2", v1func), ("v1", v2func)])?;
        assert!(switched_cases.infer_type(&sig).is_err());

        Ok(())
    }

    /// Test case that the type checker should reject: polymorphic copy and discard nodes are
    /// wired so that an `int` and a `bool` type clash.
    #[test]
    fn test_ill_typed_0() -> Result<(), Box<dyn Error>> {
        let graph = {
            let mut builder = GraphBuilder::new();
            let [input, output] = Graph::boundary();

            let copy0 = builder.add_node("copy")?;
            let copy1 = builder.add_node("copy")?;
            let discard = builder.add_node("discard")?;

            builder.add_edge((input, "value"), (copy0, "value"), None)?;
            builder.add_edge((copy0, "value_0"), (copy1, "value"), None)?;
            builder.add_edge((copy0, "value_1"), (output, "output0"), Type::Bool)?;
            builder.add_edge((copy1, "value_0"), (discard, "value"), None)?;
            builder.add_edge((copy1, "value_1"), (output, "output1"), Type::Int)?;

            builder.build()?
        };

        assert!(graph.infer_type(&signature()).is_err());

        Ok(())
    }

    /// Test case that the type checker should reject: a graph output is connected to a
    /// non-existent output port on a discard node.
    #[test]
    fn test_ill_typed_1() -> Result<(), Box<dyn Error>> {
        let graph = {
            let mut builder = GraphBuilder::new();
            let [input, output] = Graph::boundary();

            let discard = builder.add_node("discard")?;
            builder.add_edge((input, "input_value"), (discard, "value"), None)?;
            builder.add_edge((discard, "value"), (output, "output_value"), None)?;
            builder.build()?
        };

        assert!(graph.infer_type(&signature()).is_err());

        Ok(())
    }

    /// Test case that the type checker should reject: taking the union of a variant
    /// type with a non-variant, or with a variant that has a conflicting choice for
    /// the same label
    #[test]
    fn test_ill_typed_variant_union() -> Result<(), Box<dyn Error>> {
        let first_type = auto_variant([("v1", Type::Float), ("v3", Type::Bool)]);
        let bad_alternatives = [
            auto_variant([("v1", Type::Int), ("v2", Type::Bool)]),
            Type::Int,
        ];
        for bad_alt in bad_alternatives {
            let graph = {
                let mut builder = GraphBuilder::new();
                let [input, output] = Graph::boundary();

                let if_ = builder.add_node("switch")?;
                builder.add_edge((input, "arg1"), (if_, "if_true"), first_type.clone())?;
                builder.add_edge((input, "arg2"), (if_, "if_false"), bad_alt)?;
                builder.add_edge((input, "which"), (if_, "pred"), Type::Bool)?;
                builder.add_edge((if_, "value"), (output, "value"), None)?;
                builder.build()?
            };
            assert!(graph.infer_type(&signature()).is_err());
        }
        Ok(())
    }

    #[test]
    fn test_unconnected_output() -> Result<(), Box<dyn Error>> {
        let [input, output] = Graph::boundary();
        let copy0;
        let graph = {
            let mut builder = GraphBuilder::new();

            copy0 = builder.add_node("copy")?;
            // let copy1 = builder.add_node( "copy")?;
            // let discard = builder.add_node( "discard")?;

            builder.add_edge((input, "value"), (copy0, "value"), Type::Int)?;
            // builder.add_edge((copy0, "value_0"), (copy1, "value"), None)?;
            builder.add_edge((copy0, "value_1"), (output, "output0"), None)?;
            // builder.add_edge((copy1, "value_0"), (discard, "value"), None)?;
            // builder.add_edge((copy1, "value_1"), (output, "output1"), Type::Int)?;

            builder.build()?
        };

        let (_, graph) = graph.infer_type(&signature())?;

        let expected_type = Type::Int;

        let actual_type = graph
            .node_input((copy0, "value"))
            .unwrap()
            .edge_type
            .as_ref()
            .unwrap();

        assert_eq!(&expected_type, actual_type);
        Ok(())
    }

    #[test]
    fn test_parallel_simple() -> Result<(), Box<dyn Error>> {
        let a: Label = TryInto::try_into("a")?;
        let b: Label = TryInto::try_into("b")?;
        let c: Label = TryInto::try_into("c")?;

        let [input, output] = Graph::boundary();
        let left_graph = {
            let mut builder = GraphBuilder::new();

            builder.add_edge((input, a), (output, a), Some(Type::Str))?;
            let const_ = builder.add_node(Value::Int(0))?;
            builder.add_edge((const_, "value"), (output, b), Some(Type::Int))?;
            builder.build()?
        };

        let right_graph = {
            let mut builder = GraphBuilder::new();
            let discard = builder.add_node("discard")?;
            builder.add_edge((input, c), (discard, "value"), Some(Type::Int))?;
            builder.build()?
        };

        let graph = {
            let mut builder = GraphBuilder::new();
            let left = builder.add_node(Value::Graph(left_graph))?;
            let right = builder.add_node(Value::Graph(right_graph))?;
            let compose = builder.add_node("parallel")?;
            builder.add_edge((left, "value"), (compose, "left"), None)?;
            builder.add_edge((right, "value"), (compose, "right"), None)?;
            builder.add_edge((compose, "value"), (output, "value"), None)?;
            builder.build()?
        };

        let (type_scheme, _) = graph.infer_type(&signature())?;

        let expected_type = {
            let mut result_type = GraphType::new();
            result_type.add_input(a, Type::Str);
            result_type.add_input(c, Type::Int);
            result_type.add_output(a, Type::Str);
            result_type.add_output(b, Type::Int);

            let mut graph_type = GraphType::new();
            graph_type.add_output(Label::value(), result_type);

            Type::Graph(graph_type)
        };

        assert_eq!(type_scheme.body, expected_type);
        assert!(type_scheme.constraints.is_empty());

        Ok(())
    }

    #[test]
    fn test_parallel_backwards() -> Result<(), Box<dyn Error>> {
        let a: Label = TryInto::try_into("a")?;
        let b: Label = TryInto::try_into("b")?;
        let c: Label = TryInto::try_into("c")?;
        let [input, output] = Graph::boundary();

        let left_graph = {
            let mut builder = GraphBuilder::new();
            builder.add_edge((input, a), (output, a), Some(Type::Str))?;
            let const_ = builder.add_node(Value::Int(0))?;
            builder.add_edge((const_, "value"), (output, b), Some(Type::Int))?;
            builder.build()?
        };

        let mut result_type = GraphType::new();
        result_type.add_input(a, Type::Str);
        result_type.add_input(c, Type::Int);
        result_type.add_output(a, Type::Str);
        result_type.add_output(b, Type::Int);

        let graph = {
            let mut builder = GraphBuilder::new();
            let left = builder.add_node(Value::Graph(left_graph))?;
            let compose = builder.add_node("parallel")?;
            builder.add_edge((left, "value"), (compose, "left"), None)?;
            builder.add_edge((input, "right"), (compose, "right"), None)?;
            builder.add_edge(
                (compose, "value"),
                (output, "value"),
                Some(result_type.clone().into()),
            )?;
            builder.build()?
        };

        let (type_scheme, _) = graph.infer_type(&signature())?;

        let expected_type = {
            let mut right_type = GraphType::new();
            right_type.add_input(c, Type::Int);

            let mut graph_type = GraphType::new();
            graph_type.add_input(<Label as TryFrom<_>>::try_from("right")?, right_type);
            graph_type.add_output(Label::value(), result_type);

            Type::Graph(graph_type)
        };

        assert!(type_scheme.constraints.is_empty());
        assert_eq!(type_scheme.body, expected_type);

        Ok(())
    }

    #[test]
    fn test_rigid_types() -> Result<(), Box<dyn Error>> {
        fn unify_flexible_with_rigid(
            flexible_type: Type,
            rigid_type: Type,
        ) -> Result<graph::Type, TypeErrors> {
            let signature = Namespace::new();
            let mut visitor = Visitor {
                constraints: ConstraintSet::new(),
                signature: &signature,
                errors: Vec::new(),
                variables: HashMap::new(),
            };
            let flexible_id = visitor.visit_type(flexible_type, &Context::empty());
            let rigid_id = visitor.visit_type_rigid(rigid_type.clone(), &Context::empty());
            visitor.constraints.add_constraint(
                Constraint::new_unify(flexible_id, rigid_id),
                Context::empty(),
            );
            let solution = solve::solve(&visitor.constraints)?;
            if visitor.errors.is_empty() {
                let soln = solution.get_type(flexible_id);
                assert_eq!(soln, rigid_type);
                Ok(soln)
            } else {
                Err(TypeErrors(visitor.errors))
            }
        }

        let a: TypeVar = TryInto::try_into("a")?;
        let b: TypeVar = TryInto::try_into("b")?;
        let c: TypeVar = TryInto::try_into("c")?;

        let diff_pair = Type::Pair(Box::new(Type::Var(a)), Box::new(Type::Var(b)));
        let same_pair = Type::Pair(Box::new(Type::Var(c)), Box::new(Type::Var(c)));
        unify_flexible_with_rigid(diff_pair.clone(), same_pair.clone())?;
        unify_flexible_with_rigid(same_pair, diff_pair).expect_err("Type mismatch");

        let list1 = Type::Vec(Box::new(Type::Var(a)));
        let list2 = Type::Vec(Box::new(Type::Var(b)));
        unify_flexible_with_rigid(list1.clone(), list2.clone())?;
        unify_flexible_with_rigid(list2, list1.clone())?;

        let concrete_list = Type::Vec(Box::new(Type::Int));
        unify_flexible_with_rigid(list1.clone(), concrete_list.clone())?;
        unify_flexible_with_rigid(concrete_list, list1).expect_err("Type mismatch");

        Ok(())
    }

    #[test]
    fn test_value_as_type() -> Result<(), Box<dyn Error>> {
        use crate::graph::RowType;
        use crate::type_checker::{value_as_type, BTreeMap, Signature};
        use std::iter::FromIterator;

        let a = TryInto::try_into("a")?;
        let b = TryInto::try_into("b")?;

        let empty_sig: Signature = Namespace::new();

        value_as_type(&Value::Int(5), Type::Int, &empty_sig)?;
        value_as_type(&Value::Float(5.1), Type::Int, &empty_sig).expect_err("Type mismatch");
        value_as_type(
            &Value::Variant(TryInto::try_into("foo")?, Box::new(Value::Float(6.7))),
            Type::Variant(RowType {
                content: BTreeMap::from([(TryInto::try_into("foo")?, Type::Float)]),
                rest: Some(TryInto::try_into("var_a")?),
            }),
            &empty_sig,
        )?;

        let [input, output] = Graph::boundary();
        let polymorphic_graph = Value::Graph({
            let mut builder = GraphBuilder::new();

            builder.add_edge((input, "arg"), (output, "value"), None)?;
            builder.build().unwrap()
        });
        let monomorphic_graph = Value::Graph({
            let mut builder = GraphBuilder::new();
            let my_add = builder.add_node("iadd")?;
            let increment = builder.add_node(Value::Int(1))?;
            builder.add_edge((input, "arg"), (my_add, "a"), None)?;
            builder.add_edge((increment, "value"), (my_add, "b"), None)?;
            builder.add_edge((my_add, "value"), (output, "value"), None)?;
            builder.build().unwrap()
        });
        let signature: Signature = {
            let mut ns = Namespace::new();
            ns.insert_into_namespace(
                "iadd".parse()?,
                GraphType {
                    inputs: RowType {
                        content: BTreeMap::from_iter([(a, Type::Int), (b, Type::Int)]),
                        rest: None,
                    },
                    outputs: RowType {
                        content: BTreeMap::from_iter([(Label::value(), Type::Int)]),
                        rest: None,
                    },
                }
                .into(),
            );
            ns
        };

        let monomorphic_type = Type::Graph(GraphType {
            inputs: RowType {
                content: BTreeMap::from_iter([(TryInto::try_into("arg")?, Type::Int)]),
                rest: None,
            },
            outputs: RowType {
                content: BTreeMap::from_iter([(Label::value(), Type::Int)]),
                rest: None,
            },
        });
        value_as_type(&polymorphic_graph, monomorphic_type.clone(), &empty_sig)?;
        value_as_type(&monomorphic_graph, monomorphic_type, &signature)?;

        let rigid_type = Type::Graph(GraphType {
            inputs: RowType {
                content: BTreeMap::from_iter([(
                    TryInto::try_into("arg")?,
                    Type::Var(TryInto::try_into("a")?),
                )]),
                rest: None,
            },
            outputs: RowType {
                content: BTreeMap::from_iter([(
                    Label::value(),
                    Type::Var(TryInto::try_into("a")?),
                )]),
                rest: None,
            },
        });

        value_as_type(&polymorphic_graph, rigid_type.clone(), &empty_sig)?;
        value_as_type(&monomorphic_graph, rigid_type, &signature).expect_err("Type mismatch");

        Ok(())
    }
}
