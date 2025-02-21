"""Implementation of simple python-only runtime."""

import asyncio
from typing import (
    Any,
    Callable,
    Iterable,
    Optional,
    TypeVar,
    cast,
)

from hugr import Hugr, InPort, Node, OutPort, ops, tys, val
from hugr.std.int import INT_T_DEF, IntVal
from hugr.val import Sum, Value

from .python_builtin import run_ext_op


class FunctionNotFound(Exception):
    """Function expected but not found."""

    def __init__(self, fname: str) -> None:
        self.function = fname
        super().__init__(f"Function {fname} not found in namespace.")


class _RuntimeState:
    h: Hugr
    edge_vals: dict[OutPort, Value]
    parent: "_RuntimeState | None"

    def __init__(self, h_or_p: "Hugr | _RuntimeState"):
        self.edge_vals = {}
        (self.h, self.parent) = (
            (h_or_p, None) if isinstance(h_or_p, Hugr) else (h_or_p.h, h_or_p)
        )

    def find(self, outp: OutPort) -> Value:
        if (v := self.edge_vals.get(outp)) is not None:
            return v
        if self.parent is None:
            raise RuntimeError(f"Not found: {outp}")
        return self.parent.find(outp)


def make_val(v: Any, ty: tys.Type) -> Value:
    if isinstance(v, Value):
        return v
    if isinstance(v, bool) and ty == tys.Bool:
        return val.TRUE if v else val.FALSE
    if isinstance(v, int):
        assert isinstance(ty, tys.ExtType)
        assert ty.type_def == INT_T_DEF
        (width_arg,) = ty.args
        assert isinstance(width_arg, tys.BoundedNatArg)
        return IntVal(v, width_arg.n)
    raise RuntimeError("Don't know how to convert python value: {v}")


# ALAN can we implement RuntimeClient somehow,
# e.g. type_check -> we support all ops as in Hugr, and can run?
class PyRuntime:
    """A simplified python-only Tierkreis runtime. Can be used with builtin
    operations and python only namespaces that are locally available."""

    def __init__(self, num_workers: int = 1):
        """Initialise with locally available namespaces, and the number of
        workers (asyncio tasks) to use in execution."""
        self.num_workers = num_workers
        self._callback: Optional[Callable[[OutPort, Value], None]] = None

    def set_callback(self, callback: Optional[Callable[[OutPort, Value], None]]):
        """Set a callback function that takes an OutPort and Value,
        which will be called every time a value is output.
        Can be used to inspect intermediate values."""
        self._callback = callback

    def callback(
        self,
        out_port: OutPort,
        val: Value,
    ):
        """If a callback function is set, call it with an edge and the value on
        the edge."""
        if self._callback:
            self._callback(out_port, val)

    async def run_graph(
        self,  # ALAN next line we've changed sig, does ignore work?
        run_g: Hugr,  # type:ignore
        *py_inputs: Any,
        fn_name: str | Node | None = None,
    ) -> list[Value]:
        """Run a tierkreis graph using the python runtime, and provided inputs.
        Returns the outputs of the graph.
        """

        if isinstance(fn_name, Node):
            main = fn_name
        elif isinstance(run_g[run_g.root].op, ops.Module):
            funcs = [
                n
                for n in run_g.children(run_g.root)
                if isinstance(run_g[n].op, ops.FuncDefn)
            ]
            if fn_name is None and len(funcs) > 1:
                fn_name = "main"
            (main,) = (
                funcs
                if fn_name is None
                else (
                    n
                    for n in funcs
                    if cast(ops.FuncDefn, run_g[n].op).f_name == fn_name
                )
            )
        else:  # Ignore fn_name...could require it to be None
            main = run_g.root
        op = run_g[main].op
        inputs: tys.TypeRow
        if isinstance(op, ops.DataflowOp):
            inputs = op.outer_signature().input
        else:
            assert isinstance(op, ops.FuncDefn)
            assert op.params == []
            inputs = op.inputs

        return await self._run_container(
            _RuntimeState(run_g),
            main,
            [make_val(p, t) for p, t in zip(py_inputs, inputs, strict=True)],
        )

    async def _run_container(
        self, st: _RuntimeState, parent: Node, inputs: list[Value]
    ) -> list[Value]:
        """parent is a DataflowOp"""
        parent_node = st.h[parent].op
        if isinstance(parent_node, (ops.DFG, ops.FuncDefn)):
            results, _ = await self._run_dataflow_subgraph(st, parent, inputs)
            return results
        if isinstance(parent_node, ops.CFG):
            pc: Node = st.h.children(parent)[0]
            last_inp_state = {}
            next_st = st
            while True:
                assert isinstance(st.h[pc].op, ops.DataflowBlock)
                if (prev_st := last_inp_state.get(pc)) is not None:
                    next_st = remove_keys_since(next_st, prev_st)
                last_inp_state[pc] = next_st
                results, next_st = await self._run_dataflow_subgraph(
                    next_st, pc, inputs
                )
                (tag, inputs) = unpack_first(*results)
                (bb,) = st.h.linked_ports(pc[tag])  # Should only be 1
                pc = bb.node
                if isinstance(st.h[pc].op, ops.ExitBlock):
                    return inputs
        if isinstance(parent_node, ops.Conditional):
            (tag, inputs) = unpack_first(*inputs)
            case_node = st.h.children(parent)[tag]
            results, _ = await self._run_dataflow_subgraph(st, case_node, inputs)
            return results
        if isinstance(parent_node, ops.TailLoop):
            while True:
                results, _ = await self._run_dataflow_subgraph(st, parent, inputs)
                (tag, inputs) = unpack_first(*results)
                if tag == BREAK_TAG:
                    return inputs
        raise RuntimeError("Unknown container type")

    async def _run_dataflow_subgraph(
        self, outer_st: _RuntimeState, parent: Node, inputs: list[Value]
    ) -> tuple[list[Value], _RuntimeState]:
        # assert isinstance(st.h[parent], ops.DfParentOp) # DfParentOp is a Protocal so no can do
        # FuncDefn corresponds to a Call, but inputs are the arguments
        st = _RuntimeState(outer_st)

        async def get_output(src: OutPort, wait: bool) -> Value:
            while wait and (src not in st.edge_vals):
                assert self.num_workers > 1
                await asyncio.sleep(0)
            return st.edge_vals[src]

        async def get_inputs(node: Node, wait: bool = True) -> list[Value]:
            return [
                await get_output(
                    _single(st.h.linked_ports(InPort(node, inp))), wait=wait
                )
                for inp in _node_inputs(st.h[node].op, False)
            ]

        async def run_node(node: Node) -> list[Value]:
            assert st.h[node].parent == parent
            tk_node = st.h[node].op

            # TODO: ops.Custom, ops.ExtOp, ops.RegisteredOp,

            if isinstance(tk_node, ops.Output):
                return []
            if isinstance(tk_node, ops.Input):
                return inputs

            if isinstance(
                tk_node,
                (ops.Const, ops.FuncDefn, ops.FuncDecl, ops.AliasDefn, ops.AliasDecl),
            ):
                # These are static only, no value outputs
                return []
            if isinstance(tk_node, ops.LoadConst):
                (const_src,) = st.h.linked_ports(InPort(node, 0))
                cst = st.h[const_src.node].op
                assert isinstance(cst, ops.Const)
                return [cst.val]
            if isinstance(tk_node, ops.LoadFunc):
                (funcp,) = st.h.linked_ports(InPort(node, 0))
                assert isinstance(st.h[funcp.node].op, ops.FuncDefn)
                assert tk_node.type_args == [], "Must monomorphize first"
                return [LoadedFunc(st.h, funcp.node)]

            inps = await get_inputs(node, wait=True)
            if isinstance(tk_node, (ops.Conditional, ops.CFG, ops.DFG, ops.TailLoop)):
                return await self._run_container(st, node, inps)
            elif isinstance(tk_node, ops.Call):
                (func_tgt,) = st.h.linked_ports(
                    InPort(node, tk_node._function_port_offset())
                )  # TODO Make this non-private?
                results, _ = await self._run_dataflow_subgraph(st, func_tgt.node, inps)
                return results
            elif isinstance(tk_node, ops.CallIndirect):
                return await do_eval(*inps)  # Function first
            elif isinstance(tk_node, ops.Tag):
                return [Sum(tk_node.tag, tk_node.sum_ty, inps)]
            elif isinstance(tk_node, ops.MakeTuple):
                return [Sum(0, tys.Sum([tk_node.types]), inps)]
            elif isinstance(tk_node, ops.UnpackTuple):
                (sum_val,) = inps
                assert isinstance(sum_val, Sum)
                return sum_val.vals
            elif isinstance(tk_node, ops.Custom):
                return run_ext_op(tk_node, inps)
            elif isinstance(tk_node, ops.AsExtOp):
                return run_ext_op(tk_node.ext_op.to_custom_op(), inps)
            else:
                raise RuntimeError("Unknown node type.")

        async def worker(queue: asyncio.Queue[Node]):
            # each worker gets the next node in the queue
            while True:
                node = await queue.get()
                # If the node is not yet runnable,
                # wait/block until it is, do not try to run any other node
                outs = await run_node(node)

                # assign outputs to edges
                assert len(outs) == _num_value_outputs(st.h[node].op)
                for outport_idx, val in enumerate(outs):
                    outp = OutPort(node, outport_idx)
                    self.callback(outp, val)
                    st.edge_vals[outp] = val
                # signal this node is now done
                queue.task_done()

        que: asyncio.Queue[Node] = asyncio.Queue(len(st.h.children(parent)))

        # add all node names to the queue in topsort order
        # if there are fewer workers than nodes, and the queue is populated
        # in a non-topsort order, some worker may just wait forever for it's
        # node's inputs to become available.
        scheduled: set[Node] = set()

        def schedule(n: Node):
            if n in scheduled:
                return
            scheduled.add(n)  # Ok as acyclic
            for inp in _node_inputs(st.h[n].op, True):
                for src in st.h.linked_ports(InPort(n, inp)):
                    if st.h[src.node].parent == parent:
                        schedule(src.node)
                    else:
                        st.edge_vals[src] = outer_st.find(src)
            que.put_nowait(n)

        for n in st.h.children(
            parent
        ):  # Input, then Output, then any unreachable from Output
            schedule(n)

        workers = [asyncio.create_task(worker(que)) for _ in range(self.num_workers)]
        queue_complete = asyncio.create_task(que.join())

        # wait for either all nodes to complete, or for a worker to return
        await asyncio.wait(
            [queue_complete, *workers], return_when=asyncio.FIRST_COMPLETED
        )
        if not queue_complete.done():
            # If the queue hasn't completed, it means one of the workers has
            # raised - find it and propagate the exception.
            # even if the rest of the graph has not completed
            for t in workers:
                if t.done():
                    t.result()  # this will raise
        for task in workers:
            task.cancel()

        # Wait until all worker tasks are cancelled.
        await asyncio.gather(*workers, return_exceptions=True)

        out_node = st.h.children(parent)[1]
        # No need to wait here, all nodes finishing executing:
        result = await get_inputs(out_node, wait=False)

        return result, st


def unpack_first(*vals: Value) -> tuple[int, list[Value]]:
    pred = vals[0]
    assert isinstance(pred, Sum)
    return (pred.tag, pred.vals + list(vals[1:]))


def _node_inputs(op: ops.Op, include_order: bool = False) -> Iterable[int]:
    if include_order:
        yield -1
    if isinstance(op, ops.DataflowOp):
        n = len(op.outer_signature().input)
        yield from range(n)
        if isinstance(op, ops.LoadFunc):
            n += 1  # Skip Function input
    elif isinstance(op, ops.Call):
        n = len(op.instantiation.input)
        yield from range(n)
        n += 1  # Skip Function input
    elif isinstance(op, (ops.Const, ops.FuncDefn)):
        n = 0
    else:
        raise RuntimeError(f"Unknown dataflow op {op}")
    if include_order:
        yield n


def _num_value_outputs(op: ops.Op) -> int:
    if isinstance(op, ops.DataflowOp):
        sig = op.outer_signature()
    elif isinstance(op, ops.Call):
        sig = op.instantiation
    elif isinstance(op, (ops.Const, ops.FuncDefn)):
        return 0
    else:
        raise RuntimeError(f"Unknown dataflow op {op}")
    return len(sig.output)


T = TypeVar("T")


def _single(vals: Iterable[T]) -> T:
    (val,) = vals
    return val


BREAK_TAG = ops.Break(tys.Either([tys.Unit], [tys.Unit])).tag


def remove_keys_since(new_st: _RuntimeState, tmpl_st: _RuntimeState) -> _RuntimeState:
    """Construct a new state like `new_st` but containing only keys also in `tmpl_st`."""

    def list_frames(st: _RuntimeState) -> list[_RuntimeState]:
        lst = [] if st.parent is None else list_frames(st.parent)
        lst.append(st)
        return lst

    new_frames = list_frames(new_st)
    old_frames = list_frames(new_st)
    i = 0
    while new_frames[i] is old_frames[i]:
        i += 1
        if i == len(old_frames):
            return old_frames[-1]
        if i == len(new_frames):
            return new_frames[-1]  #
    assert i > 0
    keys_can_keep = frozenset.union(
        *(frozenset(old_frame.edge_vals.keys()) for old_frame in old_frames[i:])
    )
    condensed_frame = _RuntimeState(old_frames[i - 1])
    for new_frame in new_frames[i:]:
        condensed_frame.edge_vals.update(
            {k: v for k, v in new_frame.edge_vals.items() if k in keys_can_keep}
        )
    return condensed_frame


from dataclasses import dataclass


@dataclass
class LoadedFunc(val.ExtensionValue):
    h: Hugr
    n: Node

    def to_value(self) -> val.Extension:
        raise RuntimeError("Do we have to?")

    def type_(self) -> tys.Type:
        fd = self.h[self.n].op
        assert isinstance(fd, ops.FuncDefn)
        assert fd.signature.params == []
        return fd.signature.body


async def do_eval(func: Value, *args: Value) -> list[Value]:
    if isinstance(func, LoadedFunc):
        return await PyRuntime().run_graph(func.h, *args, fn_name=func.n)
    if isinstance(func, val.Function):
        return await PyRuntime().run_graph(func.body, *args)
    raise RuntimeError(f"Don't know how to eval {func}")
