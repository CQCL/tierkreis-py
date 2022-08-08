import pytest

from tierkreis.core import Labels
from tierkreis.core.tierkreis_graph import TierkreisEdge, TierkreisGraph
from tierkreis.core.values import OptionValue, StringValue, TierkreisValue, VariantValue
from tierkreis.frontend import python_builtin
from tierkreis.frontend.python_runtime import PyRuntime
from tierkreis.frontend.runtime_client import ServerRuntime


def _loop_graph() -> TierkreisGraph:
    ifg = TierkreisGraph()
    ifg.set_outputs(value=ifg.add_tag(Labels.BREAK, value=ifg.input["x"]))

    elg = TierkreisGraph()
    elg.set_outputs(
        value=elg.add_tag(
            Labels.CONTINUE,
            value=elg.add_func("builtin/iadd", a=elg.input["x"], b=1),
        )
    )

    tg = TierkreisGraph()
    tg.set_outputs(
        value=tg.add_func(
            "builtin/eval",
            thunk=tg.add_func(
                "builtin/switch",
                pred=tg.add_func("builtin/igt", a=tg.input["value"], b=5),
                if_true=ifg,
                if_false=elg,
            ),
            x=tg.copy_value(tg.input["value"]),
        )["value"]
    )
    return tg


@pytest.fixture()
def sample_graph() -> TierkreisGraph:
    one_graph = TierkreisGraph()
    one_graph.set_outputs(
        value=one_graph.add_func(
            "builtin/iadd", a=one_graph.input["value"], b=one_graph.input["other"]
        )
    )
    many_graph = TierkreisGraph()
    many_graph.discard(many_graph.input["other"])
    many_graph.set_outputs(
        value=many_graph.add_func("builtin/id", value=many_graph.input["value"])
    )

    tg = TierkreisGraph()
    tg.set_outputs(
        out=tg.input["in"],
        b=tg.add_func("builtin/iadd", a=1, b=3),
        tag=tg.add_tag("boo", value="world"),
        add=tg.add_func("python_nodes/python_add", a=23, b=123),
        _and=tg.add_func("builtin/and", a=True, b=False),
        result=tg.add_func(
            "builtin/eval",
            thunk=tg.add_match(
                tg.input["vv"],
                one=tg.add_const(one_graph),
                many=tg.add_const(many_graph),
            )["thunk"],
            other=2,
        ),
        loop_out=tg.add_func("builtin/loop", body=_loop_graph(), value=2)["value"],
        option=tg.add_func(
            "builtin/unwrap", option=OptionValue(StringValue("inside_option"))
        ),
    )
    return tg


@pytest.mark.asyncio
async def test_run_graph(
    server_client: ServerRuntime, sample_graph: TierkreisGraph, pyruntime: PyRuntime
):

    ins = {"in": "hello", "vv": VariantValue("one", TierkreisValue.from_python(1))}
    out_py = await pyruntime.run_graph(
        sample_graph,
        ins,
    )

    out_rs = await server_client.run_graph(sample_graph, ins)
    assert out_rs == out_py


@pytest.mark.asyncio
async def test_callback(sample_graph: TierkreisGraph, pyruntime: PyRuntime):
    ins = {"in": "world", "vv": VariantValue("many", TierkreisValue.from_python(2))}

    cache = {}

    def cback(e: TierkreisEdge, v: TierkreisValue):
        cache[e] = v

    pyruntime.set_callback(cback)
    outs = await pyruntime.run_graph(sample_graph, ins)
    assert all(e in cache for e in sample_graph.edges())
    assert sorted(outs) == sorted(sample_graph.outputs())


@pytest.mark.asyncio
async def test_builtin_signature(server_client: ServerRuntime):
    # TODO test all the implementations as well!
    remote_ns = (await server_client.get_signature())["builtin"].functions
    assert remote_ns.keys() == python_builtin.namespace.functions.keys()
    for f, tkfunc in python_builtin.namespace.functions.items():
        remote_func = remote_ns[f]
        assert remote_func == tkfunc.declaration
