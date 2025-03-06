use crate::operations::RuntimeOperation;
use anyhow::{anyhow, Context};
use std::collections::HashMap;
use tierkreis_core::{
    namespace::{ConflictingDeclaration, FunctionDeclaration, Namespace},
    prelude::TryInto,
    symbol::{FunctionName, Name},
};

type OperationFactory = Box<dyn Fn() -> RuntimeOperation + Send + Sync>;

/// Worker for functions that run locally in the runtime's process.
pub struct LocalWorker {
    signature: Namespace<(FunctionDeclaration, OperationFactory)>,
}

impl LocalWorker {
    /// Creates a new instance with no functions (yet). See [add_function](Self::add_function)
    pub fn new() -> Self {
        Self {
            signature: Default::default(),
        }
    }

    /// Lists all the [FunctionDeclaration]s this worker provides
    pub fn declarations(&self) -> Namespace<FunctionDeclaration> {
        self.signature.map_ref(|x| x.0.clone())
    }

    /// Adds a function to the local worker.
    pub fn add_function<F>(&mut self, name: FunctionName, entry: FunctionDeclaration, run: F)
    where
        F: Fn() -> RuntimeOperation + Send + Sync + 'static,
    {
        self.signature
            .insert_into_namespace(name, (entry, Box::new(run)));
    }

    /// A local worker with all builtin operations.
    pub fn builtins() -> Self {
        use crate::operations::*;
        use tierkreis_core::builtins::*;
        let operations = [
            ("id", operation_id as fn() -> RuntimeOperation),
            ("sleep", operation_sleep),
            ("eval", operation_eval),
            ("make_pair", operation_make_pair),
            ("unpack_pair", operation_unpack_pair),
            ("switch", operation_switch),
            ("copy", operation_copy),
            ("discard", operation_discard),
            ("eq", operation_equality),
            ("neq", operation_not_equality),
            ("push", operation_push),
            ("pop", operation_pop),
            ("loop", operation_loop),
            ("sequence", operation_sequence),
            ("make_struct", operation_make_struct),
            ("unpack_struct", operation_unpack_struct),
            ("insert_key", operation_insert_key),
            ("remove_key", operation_remove_key),
            ("int_to_float", operation_int_to_float),
            ("float_to_int", operation_float_to_int),
            ("partial", operation_partial),
            ("parallel", operation_parallel),
            ("map", operation_map),
            /*
                INTEGER ARITHMETIC OPERATIONS
            */
            ("iadd", || binary_int_operation(|a, b| a + b)),
            ("isub", || binary_int_operation(|a, b| a - b)),
            ("imul", || binary_int_operation(|a, b| a * b)),
            ("idiv", || {
                binary_int_operation_with_error(|a, b| {
                    a.checked_div(b).context("Tried to divide by zero.")
                })
            }),
            ("imod", || {
                binary_int_operation_with_error(|a, b| {
                    a.checked_rem_euclid(b)
                        .context("Tried to take modulus with zero.")
                })
            }),
            ("ipow", || {
                binary_int_operation_with_error(|a, b| {
                    let b_unsigned: u32 = TryInto::try_into(b)
                        .map_err(|_| anyhow!("Input b to ipow must be positive integer."))?;
                    Ok(a.pow(b_unsigned))
                })
            }),
            /*
                FLOAT ARITHMETIC OPERATIONS
            */
            ("fadd", || binary_flt_operation(|a, b| a + b)),
            ("fsub", || binary_flt_operation(|a, b| a - b)),
            ("fmul", || binary_flt_operation(|a, b| a * b)),
            ("fdiv", || binary_flt_operation(|a, b| a / b)),
            ("fmod", || binary_flt_operation(|a, b| a % b)),
            ("fpow", || binary_flt_operation(|a, b| a.powf(b))),
            /*
                BINARY INT COMPARISON FUNCTIONS
            */
            ("ilt", || binary_int_comparison(|a, b| a < b)),
            ("ileq", || binary_int_comparison(|a, b| a <= b)),
            ("igt", || binary_int_comparison(|a, b| a > b)),
            ("igeq", || binary_int_comparison(|a, b| a >= b)),
            /*
                BINARY FLOAT COMPARISON FUNCTIONS
            */
            ("flt", || binary_flt_comparison(|a, b| a < b)),
            ("fleq", || binary_flt_comparison(|a, b| a <= b)),
            ("fgt", || binary_flt_comparison(|a, b| a > b)),
            ("fgeq", || binary_flt_comparison(|a, b| a >= b)),
            /*
                BOOLEAN OPERATIONS
            */
            ("and", || binary_bool_operation(|a, b| a && b)),
            ("or", || binary_bool_operation(|a, b| a || b)),
            ("xor", || binary_bool_operation(|a, b| a ^ b)),
        ];
        let mut ns: HashMap<_, FunctionDeclaration> = namespace().functions;

        let mut worker = Self::new();

        for (name, op) in operations {
            let n: Name = tierkreis_core::prelude::TryFrom::try_from(name).unwrap();
            worker.add_function(FunctionName::builtin(n), ns.remove(&n).unwrap(), op);
        }
        worker
    }

    /// Merges all the functions from another worker into this one
    pub fn merge(&mut self, other: LocalWorker) -> Result<(), ConflictingDeclaration> {
        self.signature.merge_namespace(other.signature, |_, _| None)
    }

    /// Gets a [RuntimeOperation] that can asynchronously execute the named function, if known
    pub fn spawn(&self, function: &FunctionName) -> Option<RuntimeOperation> {
        self.signature.get(function).map(|f| f.1())
    }
}

impl Default for LocalWorker {
    fn default() -> Self {
        Self::new()
    }
}
