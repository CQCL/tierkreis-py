from datetime import datetime
import json
import os
from pathlib import Path

import qnexus as qnx
from qnexus.client.utils import write_token
from qnexus.models import AerConfig
from qnexus.models.references import ExecuteJobRef

from pytket._tket.circuit import Circuit
from pytket.backends.status import StatusEnum

from models import NodeDefinition
from tierkreis.core.protos.tierkreis.v1alpha1.graph import Value


def setup_project():
    refresh_token = os.environ.get("NEXUS_REFRESH_TOKEN")
    assert refresh_token is not None
    write_token("refresh_token", refresh_token)

    project = qnx.projects.get_or_create(name="Riken-Test")
    qnx.context.set_active_project(project)


def submit(circuit: Circuit) -> ExecuteJobRef:
    setup_project()
    config = AerConfig()
    identifier = datetime.now().timestamp()
    upload_ref = qnx.circuits.upload(circuit=circuit, name=f"circuit-{identifier}")
    return qnx.start_execute_job(
        circuits=[upload_ref],
        n_shots=[100],
        backend_config=config,
        name=f"execution-{identifier}",
    )


def check_status(job_ref: ExecuteJobRef) -> StatusEnum:
    setup_project()
    return qnx.jobs.status(job_ref).status


def get_result(job_ref: ExecuteJobRef) -> dict[str, int]:
    setup_project()
    ref_result = qnx.jobs.results(job_ref)[0]
    backend_result = ref_result.download_result()
    counter = backend_result.get_counts()
    return {str(k): int(v) for k, v in counter.items()}


def run(node_definition: NodeDefinition):
    name = node_definition.function_name
    print(node_definition)
    if name == "./examples/nexus-worker/submit":
        with open(node_definition.inputs["circuit"], "rb") as fh:
            circuit_str = Value.FromString(fh.read()).str
            circuit = Circuit.from_dict(json.loads(circuit_str))  # type:ignore

        execute_ref = submit(circuit)

        with open(node_definition.outputs["execute_ref"], "wb+") as fh:
            fh.write(Value(str=execute_ref.model_dump_json()).SerializeToString())

    elif name == "./examples/nexus-worker/check_status":
        with open(node_definition.inputs["execute_ref"], "rb") as fh:
            z = fh.read()
            ref_str = Value.FromString(z).str
            execute_ref = ExecuteJobRef(**json.loads(ref_str))

        status_enum = check_status(execute_ref)

        with open(node_definition.outputs["status_enum"], "wb+") as fh:
            fh.write(Value(str=status_enum.name).SerializeToString())

    elif name == "./examples/nexus-worker/get_result":
        with open(node_definition.inputs["execute_ref"], "rb") as fh:
            ref_str = Value.FromString(fh.read()).str
            execute_ref = ExecuteJobRef(**json.loads(ref_str))

        distribution = get_result(execute_ref)

        with open(node_definition.outputs["distribution"], "wb+") as fh:
            fh.write(Value(str=json.dumps(distribution)).SerializeToString())

    else:
        raise ValueError(f"nexus-worker: unknown function: {name}")

    Path(node_definition.done_path).touch()
