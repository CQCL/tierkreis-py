import json
from pathlib import Path
import shutil
from time import sleep
from uuid import UUID, uuid4
from models import NodeDefinition
from main import run

from pytket._tket.circuit import Circuit


def get_circ() -> Circuit:
    """Build a test circuit."""
    circ = Circuit(2, 2)
    circ.Rx(0.2, 0).CX(0, 1).Rz(-0.7, 1).measure_all()
    return circ


def test_submit_poll():
    tmp_path = Path(f"/tmp/nexus-worker/{UUID(int=0)}")
    if tmp_path.exists():
        shutil.move(tmp_path, f"/tmp/{uuid4()}")
    tmp_path.mkdir(parents=True, exist_ok=True)

    with open(tmp_path / "circuit", "w") as fh:
        fh.write(json.dumps(get_circ().to_dict()))  # type: ignore

    node_0_path = tmp_path / "N0"
    node_0_path.mkdir(parents=True, exist_ok=True)
    node_definition = NodeDefinition(
        function_name="submit",
        inputs={"circuit": tmp_path / "circuit"},
        outputs={"execute_ref": node_0_path / "execute_ref"},
        done_path=node_0_path / "_done",
    )
    run(node_definition)

    for i in range(100):
        node_path = tmp_path / f"N{i+2}"
        node_path.mkdir(parents=True, exist_ok=True)
        node_definition = NodeDefinition(
            function_name="check_status",
            inputs={"execute_ref": node_0_path / "execute_ref"},
            outputs={"status_enum": node_path / "status_enum"},
            done_path=node_path / "_done",
        )
        run(node_definition)

        with open(node_path / "status_enum", "r") as fh:
            status_str = fh.read()
            if status_str in ["COMPLETED", "CANCELLED", "ERROR"]:
                break

        sleep(5)

    node_1_path = tmp_path / "N1"
    node_1_path.mkdir(parents=True, exist_ok=True)
    node_definition = NodeDefinition(
        function_name="get_result",
        inputs={"execute_ref": node_0_path / "execute_ref"},
        outputs={"distribution": node_1_path / "distribution"},
        done_path=node_1_path / "_done",
    )
    run(node_definition)
