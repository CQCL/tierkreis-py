import json
from logging import getLogger
from pathlib import Path
from sys import argv

from pydantic import BaseModel

from tierkreis.core.protos.tierkreis.v1alpha1.graph import Value

logger = getLogger(__name__)


class NodeDefinition(BaseModel):
    function_name: str
    inputs: dict[str, Path]
    outputs: dict[str, Path]
    done_path: Path


def iadd(a: int, b: int) -> int:
    logger.debug(f"iadd {a} {b}")
    return a + b


def itimes(a: int, b: int) -> int:
    logger.debug(f"itimes {a} {b}")
    return a * b


def igt(a: int, b: int) -> bool:
    logger.debug(f"igt {a} {b}")
    return a > b


def run(node_definition: NodeDefinition):
    logger.debug(node_definition)
    if node_definition.function_name == "iadd":
        with open(node_definition.inputs["a"], "rb") as fh:
            a: int = json.loads(fh.read())
        with open(node_definition.inputs["b"], "rb") as fh:
            b: int = json.loads(fh.read())

        c = iadd(a, b)

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(c))

    elif node_definition.function_name == "itimes":
        with open(node_definition.inputs["a"], "rb") as fh:
            a: int = json.loads(fh.read())
        with open(node_definition.inputs["b"], "rb") as fh:
            b: int = json.loads(fh.read())

        c = itimes(a, b)

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(c))

    elif node_definition.function_name == "igt":
        with open(node_definition.inputs["a"], "rb") as fh:
            a: int = json.loads(fh.read())
        with open(node_definition.inputs["b"], "rb") as fh:
            b: int = json.loads(fh.read())

        c = igt(a, b)

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(c))

    elif node_definition.function_name == "and":
        with open(node_definition.inputs["a"], "rb") as fh:
            a = json.loads(fh.read())
        with open(node_definition.inputs["b"], "rb") as fh:
            b = json.loads(fh.read())

        c = a and b

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(c))

    elif node_definition.function_name == "id":
        with open(node_definition.inputs["value"], "rb") as fh:
            value = json.loads(fh.read())

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(value))

    elif node_definition.function_name == "str_eq":
        with open(node_definition.inputs["a"], "rb") as fh:
            str_a = json.loads(fh.read())

        with open(node_definition.inputs["b"], "rb") as fh:
            str_b = json.loads(fh.read())

        ans = str_a == str_b

        with open(node_definition.outputs["value"], "w+") as fh:
            fh.write(json.dumps(ans))
    else:
        raise ValueError(f"function name {node_definition.function_name} not found")
    node_definition.done_path.touch()


if __name__ == "__main__":
    worker_definition_path = argv[1]
    with open(worker_definition_path, "r") as fh:
        node_definition = NodeDefinition(**json.loads(fh.read()))

    run(node_definition)
