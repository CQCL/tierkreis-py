#!/usr/bin/env python3
import asyncio
import base64
from dataclasses import dataclass
from typing import Generic, Optional, TypeVar, cast

from tierkreis import TierkreisGraph
from tierkreis.client.server_client import RuntimeClient
from tierkreis.core.python import RuntimeGraph
from tierkreis.core.types import StarKind, UnpackRow
from tierkreis.worker import Namespace
from tierkreis.worker.prelude import start_worker_server

root = Namespace()
namespace = root["python_nodes"]
subspace = namespace["subspace"]

A = TypeVar("A")


@namespace.function(type_vars={A: StarKind()})
async def id_py(value: A) -> A:
    "Identity function which passes on the value on port 'value'."
    return value


@namespace.function()
async def increment(value: int) -> int:
    return value + 1


@subspace.function(name="increment")
async def increment_subspace(value: int) -> int:
    # Intentionally give a different definition in the subspace
    # to check names are being resolved correctly
    return value + 2


# This deliberately has the wrong python type annotation because
# the TierkreisFunction's types are copied from there. Thus, this
# produces a TierkreisFunction which claims to be of type Int->Int,
# even though it actually produces floats.
@namespace.function()
async def mistyped_op(inp: int) -> int:
    return inp + 1.1  # type: ignore


@namespace.function()
async def python_add(a: int, b: int) -> int:
    return a + b


@dataclass
class IdDelayInputs(UnpackRow, Generic[A]):
    wait: int
    value: A


@dataclass
class IdDelayOutputs(UnpackRow, Generic[A]):
    value: A


@namespace.function(
    type_vars={A: StarKind()},
)
async def id_delay(inputs: IdDelayInputs[A]) -> IdDelayOutputs[A]:
    await asyncio.sleep(inputs.wait)
    return IdDelayOutputs(value=inputs.value)


@namespace.function(type_vars={A: StarKind()})
async def fail(value: A) -> A:
    raise RuntimeError("fail node was run")


@namespace.function()
async def test_option(x: Optional[int]) -> int:
    if x is None:
        return -1
    # Just check the runtime values correspond with the type annotations
    assert isinstance(x, int)
    return x


@namespace.function(type_vars={cast(TypeVar, A): StarKind()}, callback=True)
async def id_with_callback(client: RuntimeClient, value: A) -> A:
    """Callback to runtime via channel to run identity"""

    tg = TierkreisGraph()
    tg.set_outputs(out=tg.input["inp"])

    # async with channel as channel:
    #     rc = RuntimeClient(channel)
    outs = await client.run_graph(tg, inp=value)

    return cast(A, outs["out"])


@dataclass
class GraphInOut(UnpackRow, Generic[A]):
    value: A


@namespace.function(type_vars={cast(TypeVar, A): StarKind()}, callback=True)
async def do_callback(
    client: RuntimeClient, graph: RuntimeGraph[GraphInOut, GraphInOut], value: A
) -> A:
    """Callback to runtime via channel to run graph provided"""
    g = graph.graph

    outs = await client.run_graph(g, value=value)

    return cast(A, outs["value"])


@dataclass
class IntStruct:
    y: int


@dataclass
class StructWithUnion:
    x: IntStruct | float


@dataclass
class UnionStructOutput(UnpackRow):
    value: StructWithUnion


@namespace.function()
async def id_union_struct(x: StructWithUnion) -> UnionStructOutput:
    return UnionStructOutput(x)


@namespace.function()
async def id_union(x: int | float) -> float | int:
    return x


@namespace.function(pass_stack=True)
async def dump_stack(stack: bytes, label: str) -> str:
    """Returns the stack trace formatted as a Base64 string prefixed with the given label"""
    stack_str: str = base64.b64encode(stack).decode("ascii")
    return f"{label} {stack_str}"


if __name__ == "__main__":
    start_worker_server("test_worker", root)
