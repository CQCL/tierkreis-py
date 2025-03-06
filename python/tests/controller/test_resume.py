import json
from pathlib import Path
from time import sleep
from uuid import UUID

from tests.sample_graph import sample_graph_without_match
from tierkreis.controller.executor.shell_executor import ShellExecutor
from tierkreis.controller.models import NodeLocation
from tierkreis.controller.resume import resume
from tierkreis.controller.start import start
from tierkreis.controller.storage.filestorage import ControllerFileStorage
from tierkreis.core import Labels
from tierkreis.core.function import FunctionName
from tierkreis.core.tierkreis_graph import FunctionNode

from pytket._tket.circuit import Circuit

root_loc = NodeLocation(location=[])


def get_circ_str() -> str:
    """Build a test circuit."""
    circ = Circuit(2, 2)
    circ.Rx(0.2, 0).CX(0, 1).Rz(-0.7, 1).measure_all()
    return json.dumps(circ.to_dict())


def test_resume_sample_graph():
    g = sample_graph_without_match()
    storage = ControllerFileStorage(UUID(int=0))
    executor = ShellExecutor(Path("./examples/launchers"), Path("./_stderr"))

    storage.clean_graph_files()
    inp_loc = storage.add_input("inp", json.dumps(4).encode())
    value_loc = storage.add_input("value", json.dumps(2).encode())
    thunk_loc = storage.add_input(Labels.THUNK, g.to_proto().SerializeToString())
    start(
        storage,
        executor,
        root_loc,
        FunctionNode(FunctionName("eval")),
        {
            "inp": (inp_loc, "inp"),
            Labels.THUNK: (thunk_loc, Labels.THUNK),
            "value": (value_loc, "value"),
        },
        g.outputs(),
    )

    for _ in range(400):
        resume(storage, executor, root_loc)
        if storage.is_node_finished(root_loc):
            break
        sleep(0.1)

    c = storage.read_output(root_loc, "loop_out")
    assert c == b"6"
