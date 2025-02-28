import asyncio
import inspect
from typing import AsyncIterator

import pytest
from grpclib.client import Channel

from tests.test_worker import main
from tierkreis.builder import Namespace
from tierkreis.client.runtime_client import RuntimeClient
from tierkreis.client.server_client import ServerRuntime
from tierkreis.core.signature import Signature
from tierkreis.core.tierkreis_graph import TierkreisGraph
from tierkreis.core.type_inference import _TYPE_CHECK
from tierkreis.pyruntime import PyRuntime


def pytest_addoption(parser):
    parser.addoption(
        "--host",
        action="store",
        help="Remote server host",
    )
    parser.addoption(
        "--port",
        action="store",
        help="Server port",
    )
    parser.addoption(
        "--pytket",
        action="store_true",
        default=False,
        help="Run pytket integration tests",
    )
    parser.addoption(
        "--client-only",
        action="store_true",
        default=False,
        help="Only run tests that use the client fixture",
    )


def pytest_runtest_setup(item):
    if not _TYPE_CHECK:
        for _ in item.iter_markers(name="skip_typecheck"):
            pytest.skip("Test skipped because typecheck is not installed")


def pytest_collection_modifyitems(config, items):
    if not config.getoption("--client-only"):
        return
    skip_non_client = pytest.mark.skip(reason="Doesn't use client fixture")
    for item in items:
        # item.fixturenames includes all transitively-required fixtures too,
        # hence check the directly-declared arguments to the function
        if "client" in inspect.signature(item.function).parameters:
            assert "client" in item.fixturenames
        else:
            item.add_marker(skip_non_client)


def pytest_configure(config):
    config.addinivalue_line(
        "markers",
        "skip_typecheck: this mark skips tests that"
        " require typecheck installed if it is not installed.",
    )

    if not config.option.pytket:
        setattr(config.option, "markexpr", "not pytket")


@pytest.fixture(scope="session")
def event_loop(request):
    loop = asyncio.get_event_loop_policy().new_event_loop()
    yield loop
    loop.close()


@pytest.fixture(scope="session")
def pyruntime():
    return PyRuntime([main.root])


@pytest.fixture(scope="function")
def pyruntime_function():
    runtime = PyRuntime([main.root])
    return runtime


@pytest.fixture(scope="session")
async def client(request, pyruntime) -> AsyncIterator[RuntimeClient]:
    host = request.config.getoption("--host", "")
    port = request.config.getoption("--port", "")

    if host and port:
        async with Channel(host, int(port)) as c:
            yield ServerRuntime(c)
    else:
        yield pyruntime


@pytest.fixture()
async def sig(client: RuntimeClient) -> Signature:
    return await client.get_signature()


@pytest.fixture()
def bi(sig: Signature) -> Namespace:
    return Namespace(sig)


@pytest.fixture()
def idpy_graph() -> TierkreisGraph:
    tk_g = TierkreisGraph()

    id_node = tk_g.add_func("python_nodes::id_py", value=tk_g.input["id_in"])
    tk_g.set_outputs(id_out=id_node)

    return tk_g
