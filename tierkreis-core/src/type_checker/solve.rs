use super::union_find::UnionFind;
use super::{
    Bound, Constraint, ConstraintId, ConstraintSet, RowType, Type, TypeError, TypeErrors, TypeId,
};
use crate::graph::{self, Kind};
use crate::symbol::{Label, TypeVar};
use std::collections::{BTreeMap, HashSet};
use std::hash::Hash;
use thiserror::Error;

pub(super) struct Solution(Solver);

impl Solution {
    pub fn get_type(&self, type_id: TypeId) -> graph::Type {
        // TODO: Type variables that occur in an interior type annotation but not in the overall
        // type of a value should be considered ambiguous and rejected. The type extraction phase
        // could be implemented more cleanly by having a separate type for that phase.

        let type_id = self.0.canonicalize(type_id);

        match self.0.get_type(type_id) {
            Type::Var(_) => graph::Type::Var(type_id_to_var(type_id)),
            // For RigidVars, preserve the external TypeVar as if it were any other concrete type
            Type::RigidVar(var) => graph::Type::Var(*var),
            Type::Int => graph::Type::Int,
            Type::Bool => graph::Type::Bool,
            Type::Str => graph::Type::Str,
            Type::Float => graph::Type::Float,
            Type::Vector(element) => {
                let element = Box::new(self.get_type(*element));
                graph::Type::Vec(element)
            }
            Type::Pair(first, second) => {
                let first = Box::new(self.get_type(*first));
                let second = Box::new(self.get_type(*second));
                graph::Type::Pair(first, second)
            }
            Type::Map(key, val) => {
                let key = Box::new(self.get_type(*key));
                let val = Box::new(self.get_type(*val));
                graph::Type::Map(key, val)
            }
            Type::Graph(graph) => {
                let inputs = match self.get_type(graph.inputs) {
                    graph::Type::Row(row) => row,
                    graph::Type::Var(var) => var.into(),
                    _ => unreachable!(),
                };
                let outputs = match self.get_type(graph.outputs) {
                    graph::Type::Row(row) => row,
                    graph::Type::Var(var) => var.into(),
                    _ => unreachable!(),
                };
                graph::Type::Graph(graph::GraphType { inputs, outputs })
            }
            Type::Row(_) => {
                let row = self.0.merged_row(type_id);

                let content = row
                    .content
                    .into_iter()
                    .map(|(label, type_)| (label, self.get_type(type_)))
                    .collect();

                let rest = match self.0.get_type(row.rest) {
                    Type::Var(_) => Some(type_id_to_var(row.rest)),
                    Type::EmptyRow => None,
                    Type::RigidVar(v) => Some(*v),
                    _ => unreachable!(),
                };

                let row = graph::RowType { content, rest };

                graph::Type::Row(row)
            }

            Type::Struct(row, name) => {
                let row = match self.get_type(*row) {
                    graph::Type::Row(row) => row,
                    graph::Type::Var(var) => var.into(),
                    _ => unreachable!(),
                };

                graph::Type::Struct(row, name.clone())
            }
            Type::EmptyRow => graph::Type::Row(graph::RowType {
                content: BTreeMap::new(),
                rest: None,
            }),
            Type::Variant(row) => {
                let row = match self.get_type(*row) {
                    graph::Type::Row(row) => row,
                    graph::Type::Var(var) => var.into(),
                    _ => unreachable!(),
                };

                graph::Type::Variant(row)
            }
        }
    }

    pub fn bounds(&self) -> impl Iterator<Item = graph::Constraint> + '_ {
        self.0.bounds().map(move |bound| self.get_bound(bound))
    }

    pub fn get_bound(&self, bound: Bound) -> graph::Constraint {
        match bound {
            Bound::Lacks(row, label) => {
                let row = self.get_type(row);
                graph::Constraint::Lacks { row, label }
            }
            Bound::Partition(r, s, t) => {
                let left = self.get_type(r);
                let right = self.get_type(s);
                let union = self.get_type(t);
                graph::Constraint::Partition { left, right, union }
            }
        }
    }

    pub fn variables(&self) -> impl Iterator<Item = (TypeVar, Kind)> + '_ {
        self.0
            .roots()
            .filter_map(move |id| match self.0.get_type(id) {
                Type::Var(kind) => Some((type_id_to_var(id), *kind)),
                _ => None,
            })
    }
}

fn type_id_to_var(id: TypeId) -> TypeVar {
    TypeVar::symbol(format!("var{}", id.0))
}

/// Attempts to solve a set of constraints.
///
/// In the case that the set of constraints is unsatisfiable, the algorithm from [A Practical
/// Framework for Type Inference Error Explanation] is used to compute a set of type errors.
///
/// [A Practical Framework for Type Inference Error Explanation]: https://dl.acm.org/doi/10.1145/2983990.2983994
pub(super) fn solve(constraints: &ConstraintSet) -> Result<Solution, TypeErrors> {
    // We attempt to solve the constraint set and disable constraints until the constraint system
    // is solvable. To this end we keep track of a family of unsatisfiable cores, i.e. sets of
    // constraints that the solver reported to be unsatisfiable together. In each iteration
    // we disable a set of constraints which non-trivially intersects all unsatisfiable cores
    // that we have found so far.
    let mut unsat_cores: Vec<HashSet<ConstraintId>> = Vec::new();

    let (solution, disabled) = loop {
        let disabled = hitting_set(unsat_cores.clone());

        match Solver::solve(constraints, &disabled) {
            Ok(solution) => break (solution, disabled),
            Err(error) => unsat_cores.push(error.unsat_core),
        }
    };

    if disabled.is_empty() {
        // If we came to a solution without disabling any constraints, we can report success.
        Ok(solution)
    } else {
        // Otherwise each disabled constraint can be converted to a type error. Since in the
        // last operation the solver succeeded, we can use the `Solution` it produced in order
        // to figure out the type errors. This way the reported types incorporate as much
        // information from the satisfiable parts of the constraint system as possible.
        let errors = disabled
            .into_iter()
            .map(|constraint_id| {
                let data = constraints.constraint(constraint_id).unwrap();
                let context = data.context.path();

                match &data.constraint {
                    Constraint::Unify { expected, found } => TypeError::Unify {
                        expected: solution.get_type(*expected),
                        found: solution.get_type(*found),
                        context,
                    },
                    Constraint::Bound(bound) => TypeError::Bound {
                        bound: solution.get_bound(*bound),
                        context,
                    },
                }
            })
            .collect();
        Err(TypeErrors(errors))
    }
}

/// Computes an approximate solution to the hitting set problem.
///
/// Given a family of sets, determine a minimal set which non-trivially intersects with each set in
/// the family. The hitting set problem is NP-hard, so this function computes a greedy approximation.
/// See [A Practical Framework for Type Inference Error Explanation] for more details
///
/// [A Practical Framework for Type Inference Error Explanation]: https://dl.acm.org/doi/10.1145/2983990.2983994
fn hitting_set<T>(mut family: Vec<HashSet<T>>) -> HashSet<T>
where
    T: Eq + Hash + Copy,
{
    let mut hitting_set = HashSet::new();
    let union: HashSet<T> = family.iter().flatten().copied().collect();

    while !family.is_empty() {
        // todo: optimize this?
        let element = union
            .iter()
            .copied()
            .max_by_key(|element| family.iter().filter(|set| set.contains(element)).count())
            .unwrap();

        family.retain(|set| !set.contains(&element));
        hitting_set.insert(element);
    }

    hitting_set
}

/// Solver for [`ConstraintSet`]s.
///
/// The solver uses a union-find data structure to compute unifications. Each type is tagged
/// with a set of constraints that mentioned the type and the sets are merged when types
/// are unified. In the case that a constraint fails, this information is used to determine
/// an unsatisfiable core.
struct Solver {
    /// Union find data structure used for efficient unification of types.
    type_ids: UnionFind,

    /// For each type id: the type that this id is bound to.
    type_terms: Vec<Type>,

    /// For each type id: The constraints that have directly influenced this type id.
    ///
    /// This is used for error reporting to figure out which constraints could have caused a type
    /// error. Note that for this purpose we need to gather all the constraints that influenced
    /// the id directly or any of the types that contain it.
    type_touched: Vec<Vec<ConstraintId>>,

    /// For each type id: The bounds that directly contain the type id.
    ///
    /// When a bound is tracked here but not in the store, it is considered as resolved and will be
    /// ignored.
    type_bounds: Vec<Vec<Bound>>,

    /// For each type id: The ids of the types that contain this type directly.
    type_parents: Vec<Vec<TypeId>>,

    /// Stack of active bounds that need to be processed.
    ///
    /// When a bound is in this stack but no longer in the store, it is treated as resolved.
    bound_active: Vec<Bound>,

    /// Store of bounds that still need to be resolved.
    ///
    /// When a bound is in the store but not in the stack of active constraints it is seen as
    /// suspended and will be woken up once more information becomes apparent about the types that
    /// it mentions.
    bound_store: HashSet<Bound>,
}

impl Solver {
    /// Attempt to solve a set of constraints. Constraints that are in the `disabled` set are
    /// ignored by the solver. In the case that the constraints are not solvable, an unsatisfiable
    /// core is returned, i.e. a set of constraints which can not be satisfied simultaneously.
    pub fn solve(
        constraints: &ConstraintSet,
        disabled: &HashSet<ConstraintId>,
    ) -> Result<Solution, SolverError> {
        let mut solver = Solver {
            type_ids: UnionFind::new(),
            type_terms: Vec::new(),
            type_touched: Vec::new(),
            type_bounds: Vec::new(),
            type_parents: Vec::new(),
            bound_active: Vec::new(),
            bound_store: HashSet::new(),
        };

        for type_ in constraints.types.iter() {
            solver.fresh_type(type_.clone());
        }

        let result = (|| -> Result<(), InternalSolverError> {
            for (id, constraint) in constraints.constraints.iter().enumerate() {
                let id = ConstraintId(id);

                if disabled.contains(&id) {
                    continue;
                }

                match constraint.constraint {
                    Constraint::Unify {
                        expected: a,
                        found: b,
                    } => {
                        solver.record_constraint(a, id);
                        solver.record_constraint(b, id);
                        solver.unify(a, b)?
                    }
                    Constraint::Bound(bound) => {
                        for type_ in bound.types() {
                            solver.record_constraint(type_, id);
                        }
                        solver.activate_bound(bound);
                    }
                }
            }

            while let Some(bound) = solver.next_active_bound() {
                solver.solve_bound(bound)?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => Ok(Solution(solver)),
            Err(err) => {
                let unsat_core = match err {
                    InternalSolverError::Mismatch(a, b) => solver.unsat_core([a, b]),
                    InternalSolverError::Cycle(a) => solver.unsat_core([a]),
                    InternalSolverError::Lacks(a, _) => solver.unsat_core([a]),
                };

                Err(SolverError { unsat_core })
            }
        }
    }

    fn fresh_type(&mut self, type_: Type) -> TypeId {
        let id = TypeId(self.type_ids.make_set());
        let children = type_.children();
        self.type_terms.push(type_);
        self.type_touched.push(Vec::new());
        self.type_bounds.push(Vec::new());
        self.type_parents.push(Vec::new());

        for child in children {
            let child = self.canonicalize(child);
            self.type_parents[child.0].push(id);
        }

        // Implied bounds
        if let Type::Row(row) = self.get_type(id).clone() {
            for label in row.content.keys() {
                self.activate_bound(Bound::Lacks(row.rest, *label));
            }
        }

        id
    }

    fn fresh_open_row<I>(&mut self, content: I) -> (TypeId, TypeId)
    where
        I: IntoIterator<Item = (Label, TypeId)>,
    {
        let rest = self.fresh_type(Type::Var(Kind::Row));
        let row = self.fresh_type(Type::Row(RowType {
            content: content.into_iter().collect(),
            rest,
        }));
        (row, rest)
    }

    fn unify(&mut self, a: TypeId, b: TypeId) -> Result<(), InternalSolverError> {
        let a = self.canonicalize(a);
        let b = self.canonicalize(b);

        if a == b {
            // The types are already equal, so we can short-circuit here.
            return Ok(());
        }

        let a_node = self.get_type(a).clone();
        let b_node = self.get_type(b).clone();

        let content = match (a_node, b_node) {
            (Type::Var(_), b_el) => {
                self.occurs_check(a, b)?;
                b_el
            }
            (a_el, Type::Var(_)) => {
                self.occurs_check(b, a)?;
                a_el
            }
            (Type::Int, Type::Int) => Type::Int,
            (Type::Bool, Type::Bool) => Type::Bool,
            (Type::Str, Type::Str) => Type::Str,
            (Type::Float, Type::Float) => Type::Float,
            (Type::Vector(a_element), Type::Vector(b_element)) => {
                self.unify(a_element, b_element)?;
                Type::Vector(a_element)
            }
            (Type::Pair(a_0, a_1), Type::Pair(b_0, b_1)) => {
                self.unify(a_0, b_0)?;
                self.unify(a_1, b_1)?;
                Type::Pair(a_0, a_1)
            }
            (Type::Map(a_0, a_1), Type::Map(b_0, b_1)) => {
                self.unify(a_0, b_0)?;
                self.unify(a_1, b_1)?;
                Type::Map(a_0, a_1)
            }
            (Type::Graph(a_graph), Type::Graph(b_graph)) => {
                self.unify(a_graph.inputs, b_graph.inputs)?;
                self.unify(a_graph.outputs, b_graph.outputs)?;
                Type::Graph(a_graph)
            }
            (Type::Row(a_row), Type::Row(b_row)) => {
                self.unify_row_types(&a_row, &b_row)?;
                Type::Row(a_row)
            }
            (Type::EmptyRow, Type::Row(row)) => {
                if row.content.is_empty() {
                    self.unify(a, row.rest)?;
                    Type::EmptyRow
                } else {
                    return Err(InternalSolverError::Mismatch(a, b));
                }
            }
            (Type::Row(row), Type::EmptyRow) => {
                if row.content.is_empty() {
                    self.unify(row.rest, b)?;
                    Type::EmptyRow
                } else {
                    return Err(InternalSolverError::Mismatch(a, b));
                }
            }
            (Type::Struct(rowa, namea), Type::Struct(rowb, nameb)) => {
                self.unify(rowa, rowb)?;
                let name = match (namea, nameb) {
                    (None, None) => None,
                    (None, Some(name)) | (Some(name), _) => Some(name),
                };
                Type::Struct(rowa, name)
            }
            (Type::Variant(a_row), Type::Variant(b_row)) => {
                self.unify(a_row, b_row)?;
                Type::Variant(a_row)
            }
            (Type::EmptyRow, Type::EmptyRow) => Type::EmptyRow,
            (_, _) => {
                return Err(InternalSolverError::Mismatch(a, b));
            }
        };

        // Union the ids of the types
        let (root, relinked) = match self.type_ids.union(a.0, b.0) {
            super::union_find::Union::Same => return Ok(()),
            super::union_find::Union::Merged(root, relinked) => (root, relinked),
        };

        // Merge the information we have about the types
        self.type_touched[root] = merge_vectors(
            std::mem::take(&mut self.type_touched[root]),
            std::mem::take(&mut self.type_touched[relinked]),
        );

        self.type_parents[root] = merge_vectors(
            std::mem::take(&mut self.type_parents[root]),
            std::mem::take(&mut self.type_parents[relinked]),
        );

        self.type_terms[root] = content;

        // Reactivate all bounds that involve the affected types
        let bounds = merge_vectors(
            std::mem::take(&mut self.type_bounds[root]),
            std::mem::take(&mut self.type_bounds[relinked]),
        );

        for bound in bounds {
            if self.bound_store.remove(&bound) {
                let bound = self.canonicalize_bound(bound);
                self.bound_store.insert(bound);

                for type_ in bound.types() {
                    self.type_bounds[type_.0].push(bound);
                }
            }
        }

        self.wake_bounds([TypeId(root)]);

        Ok(())
    }

    /// Recursively wake up all bounds that involve a type or one of its ancestors.
    fn wake_bounds(&mut self, origins: impl IntoIterator<Item = TypeId>) {
        let bounds: HashSet<_> = self
            .type_ancestors(origins)
            .flat_map(|id| self.type_bounds[id.0].iter().copied())
            .collect();

        self.bound_active.extend(bounds);
    }

    fn activate_bound(&mut self, bound: Bound) {
        let bound = self.canonicalize_bound(bound);

        if self.bound_store.insert(bound) {
            for type_ in bound.types() {
                self.type_bounds[type_.0].push(bound);
            }
        }

        self.bound_active.push(bound);
    }

    #[allow(clippy::needless_collect)]
    fn bounds_with_type(&self, type_: TypeId) -> impl Iterator<Item = Bound> {
        let bounds: Vec<_> = self.type_bounds[type_.0]
            .iter()
            .map(move |bound| self.canonicalize_bound(*bound))
            .filter(|bound| self.bound_store.contains(bound))
            .collect();
        bounds.into_iter()
    }

    fn canonicalize_bound(&self, bound: Bound) -> Bound {
        match bound {
            Bound::Lacks(row, label) => {
                let row = self.canonicalize(row);
                Bound::Lacks(row, label)
            }
            Bound::Partition(r, s, t) => {
                let r = self.canonicalize(r);
                let s = self.canonicalize(s);
                let t = self.canonicalize(t);
                Bound::Partition(r, s, t)
            }
        }
    }

    fn resolve_bound(&mut self, bound: Bound) {
        let bound = self.canonicalize_bound(bound);
        self.bound_store.remove(&bound);
    }

    fn has_bound(&mut self, bound: Bound) -> bool {
        let bound = self.canonicalize_bound(bound);
        self.bound_store.contains(&bound)
    }

    fn next_active_bound(&mut self) -> Option<Bound> {
        loop {
            match self.bound_active.pop() {
                Some(bound) => {
                    let bound = self.canonicalize_bound(bound);
                    if self.bound_store.contains(&bound) {
                        break Some(bound);
                    }
                }
                None => break None,
            }
        }
    }

    pub fn bounds(&self) -> impl Iterator<Item = Bound> + '_ {
        self.bound_store.iter().copied()
    }

    fn solve_bound(&mut self, bound: Bound) -> Result<(), InternalSolverError> {
        match bound {
            Bound::Lacks(row_id, label) => self.solve_lacks_bound(row_id, label),
            Bound::Partition(r, s, t) => self.solve_partition_bound(r, s, t),
        }
    }

    fn solve_lacks_bound(
        &mut self,
        row_id: TypeId,
        label: Label,
    ) -> Result<(), InternalSolverError> {
        match self.get_type(row_id) {
            Type::Row(row) => {
                let rest = row.rest;

                // A lacks constraint is violated if the row contains the label.
                if row.content.contains_key(&label) {
                    return Err(InternalSolverError::Lacks(row_id, label));
                }

                // Require that the rest of the row also lacks the label.
                self.resolve_bound(Bound::Lacks(row_id, label));
                self.activate_bound(Bound::Lacks(rest, label));
            }

            Type::EmptyRow => {
                // An empty row lacks all labels so we can immediately resolve the bound.
                self.resolve_bound(Bound::Lacks(row_id, label));
            }

            Type::Var(_) => {
                for bound in self.bounds_with_type(row_id) {
                    if let Bound::Partition(r, s, t) = bound {
                        if t == row_id {
                            // If the union of two rows lacks a label then each of the individual rows
                            // lacks that label as well.
                            self.activate_bound(Bound::Lacks(r, label));
                            self.activate_bound(Bound::Lacks(s, label));
                        } else if r == row_id || s == row_id {
                            // Finding out that r or s lack a label can resolve ambiguity, so we can
                            // reactivate the constraint.
                            self.activate_bound(bound);
                        }
                    }
                }
            }

            _ => {
                // TODO: Kind error
            }
        }

        Ok(())
    }

    fn solve_partition_bound(
        &mut self,
        r: TypeId,
        s: TypeId,
        t: TypeId,
    ) -> Result<(), InternalSolverError> {
        let r_data = self.get_type(r);
        let s_data = self.get_type(s);
        let t_data = self.get_type(t);

        match (r_data, s_data, t_data) {
            (Type::Row(r_row), _, _) => {
                let row = r_row.clone();

                // Since the partition constraint encodes a disjoint union, `s` can not contain any of
                // the labels present in `r`. We can communicate this using a lacks bound.
                for label in row.content.keys() {
                    self.activate_bound(Bound::Lacks(s, *label));
                }

                // `t` must contain all fields of `r`.
                let (t_expected, t_rest) = self.fresh_open_row(row.content);
                self.unify(t, t_expected)?;

                self.activate_bound(Bound::Partition(row.rest, s, t_rest));
                self.resolve_bound(Bound::Partition(r, s, t));
            }

            (_, Type::Row(s_row), _) => {
                // Analogous to the case above.
                let row = s_row.clone();

                for label in row.content.keys() {
                    self.activate_bound(Bound::Lacks(r, *label));
                }

                let (t_expected, t_rest) = self.fresh_open_row(row.content);

                self.unify(t, t_expected)?;
                self.resolve_bound(Bound::Partition(r, s, t));
                self.activate_bound(Bound::Partition(r, row.rest, t_rest));
            }

            (Type::EmptyRow, _, _) => {
                // When `r` is empty we must have `s = t`.
                self.unify(s, t)?;
                self.resolve_bound(Bound::Partition(r, s, t));
            }

            (_, Type::EmptyRow, _) => {
                // Analogous to the case above.
                self.unify(r, t)?;
                self.resolve_bound(Bound::Partition(r, s, t));
            }

            (_, _, Type::Row(_)) => {
                // When we know fields in the union `t`, we can try and move them into either `r` or
                // `s` when the ambiguity is resolved by a lacks constraint.
                let row = self.merged_row(t);

                let mut r_add = BTreeMap::new();
                let mut s_add = BTreeMap::new();
                let mut t_ambiguous = BTreeMap::new();

                for (label, type_) in row.content {
                    if self.has_bound(Bound::Lacks(r, label)) {
                        s_add.insert(label, type_);
                    } else if self.has_bound(Bound::Lacks(s, label)) {
                        r_add.insert(label, type_);
                    } else {
                        t_ambiguous.insert(label, type_);
                    }
                }

                if r_add.is_empty() && s_add.is_empty() {
                    return Ok(());
                }

                let r_rest = if r_add.is_empty() {
                    r
                } else {
                    let (r_expected, r_rest) = self.fresh_open_row(r_add);
                    self.unify(r, r_expected)?;
                    r_rest
                };

                let s_rest = if s_add.is_empty() {
                    s
                } else {
                    let (s_expected, s_rest) = self.fresh_open_row(s_add);
                    self.unify(r, s_expected)?;
                    s_rest
                };

                let t_rest = self.fresh_type(Type::Row(RowType {
                    content: t_ambiguous,
                    rest: row.rest,
                }));

                self.resolve_bound(Bound::Partition(r, s, t));
                self.activate_bound(Bound::Partition(r_rest, s_rest, t_rest));
            }

            (_, _, Type::EmptyRow) => {
                // When the union of `r` and `s` is empty, `r` and `s` need to be empty as well.
                let r_empty = self.fresh_type(Type::EmptyRow);
                self.unify(r, r_empty)?;
                let s_empty = self.fresh_type(Type::EmptyRow);
                self.unify(s, s_empty)?;
                self.resolve_bound(Bound::Partition(r, s, t));
            }

            _ => {
                // TODO: Accept variables, reject the rest as a type error
            }
        };

        Ok(())
    }

    fn unify_row_types(
        &mut self,
        a_row: &RowType,
        b_row: &RowType,
    ) -> Result<(), InternalSolverError> {
        let mut common = Vec::new();
        let mut a_missing = BTreeMap::new();
        let mut b_missing = BTreeMap::new();

        for (label, a_type) in a_row.content.iter() {
            match b_row.content.get(label) {
                Some(b_type) => common.push((label, a_type, b_type)),
                None => {
                    b_missing.insert(*label, *a_type);
                }
            }
        }

        for (label, b_type) in b_row.content.iter() {
            if !a_row.content.contains_key(label) {
                a_missing.insert(*label, *b_type);
            }
        }

        // Unify types with common labels
        for (_, a_type, b_type) in common {
            self.unify(*a_type, *b_type)?;
        }

        match (a_missing.is_empty(), b_missing.is_empty()) {
            (false, false) => {
                // Both rows have labels that the other row does not have.
                let common_rest = self.fresh_type(Type::Var(Kind::Row));

                let a_rest = self.fresh_type(Type::Row(RowType {
                    content: a_missing,
                    rest: common_rest,
                }));

                let b_rest = self.fresh_type(Type::Row(RowType {
                    content: b_missing,
                    rest: common_rest,
                }));

                self.unify(a_row.rest, a_rest)?;
                self.unify(b_row.rest, b_rest)?;
            }
            (false, true) => {
                // `a_row` is missing some labels present in `b_row`.
                let a_rest = self.fresh_type(Type::Row(RowType {
                    content: a_missing,
                    rest: b_row.rest,
                }));

                self.unify(a_row.rest, a_rest)?;
            }
            (true, false) => {
                // `b_row` is missing some labels present in `a_row`.
                let b_rest = self.fresh_type(Type::Row(RowType {
                    content: b_missing,
                    rest: a_row.rest,
                }));

                self.unify(b_row.rest, b_rest)?;
            }
            (true, true) => {
                // Both rows have the same labels.
                self.unify(a_row.rest, b_row.rest)?;
            }
        }

        Ok(())
    }

    fn canonicalize(&self, id: TypeId) -> TypeId {
        TypeId(self.type_ids.find(id.0))
    }

    fn get_type(&self, id: TypeId) -> &Type {
        let id = self.canonicalize(id);
        &self.type_terms[id.0]
    }

    fn record_constraint(&mut self, id: TypeId, constraint: ConstraintId) {
        let id = self.canonicalize(id);
        self.type_touched[id.0].push(constraint);
    }

    fn occurs_check(&self, needle: TypeId, haystack: TypeId) -> Result<(), InternalSolverError> {
        let needle = self.canonicalize(needle);
        let haystack = self.canonicalize(haystack);

        if needle == haystack {
            return Err(InternalSolverError::Cycle(needle));
        }

        for child in self.get_type(haystack).children() {
            self.occurs_check(needle, child)?;
        }

        Ok(())
    }

    fn merged_row(&self, type_id: TypeId) -> RowType {
        match self.get_type(type_id) {
            Type::Var(_) => RowType {
                content: BTreeMap::new(),
                rest: self.canonicalize(type_id),
            },
            Type::Row(row) => {
                let mut merged = self.merged_row(row.rest);
                merged
                    .content
                    .extend(row.content.iter().map(|(l, t)| (*l, *t)));
                merged
            }
            Type::EmptyRow => RowType {
                content: BTreeMap::new(),
                rest: type_id,
            },
            Type::RigidVar(_) => RowType {
                content: BTreeMap::new(),
                rest: type_id,
            },
            _ => panic!("called merged_row on a non-row type"),
        }
    }

    fn roots(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.type_ids.roots().map(TypeId)
    }

    fn type_ancestors(&self, origins: impl IntoIterator<Item = TypeId>) -> Ancestors<'_> {
        Ancestors {
            solver: self,
            pending: origins.into_iter().collect(),
            visited: HashSet::new(),
        }
    }

    /// Computes a set of constraints which directly or indirectly affected a collection of types
    /// involved in an error.
    fn unsat_core(&self, origins: impl IntoIterator<Item = TypeId>) -> HashSet<ConstraintId> {
        self.type_ancestors(origins)
            .flat_map(|id| self.type_touched[id.0].iter().copied())
            .collect()
    }
}

struct Ancestors<'a> {
    solver: &'a Solver,
    pending: Vec<TypeId>,
    visited: HashSet<TypeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = TypeId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next = self.pending.pop()?;
            let next = self.solver.canonicalize(next);

            if !self.visited.insert(next) {
                continue;
            }

            self.pending
                .extend(self.solver.type_parents[next.0].iter().copied());

            return Some(next);
        }
    }
}

#[derive(Debug, Error)]
enum InternalSolverError {
    #[error("could not unify types")]
    Mismatch(TypeId, TypeId),
    #[error("detected cycle while unifying types")]
    Cycle(TypeId),
    #[error("lacks constraint failed")]
    Lacks(TypeId, Label),
}

#[derive(Debug, Error)]
#[error("failed to solve type constraints")]
pub(super) struct SolverError {
    pub unsat_core: HashSet<ConstraintId>,
}

/// Computes a vector containing the elements of two input vectors by appending the smaller vector
/// to the larger one.
fn merge_vectors<T>(mut a: Vec<T>, mut b: Vec<T>) -> Vec<T> {
    if a.len() < b.len() {
        b.extend(a);
        b
    } else {
        a.extend(b);
        a
    }
}
