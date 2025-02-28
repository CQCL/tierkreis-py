import json
from sys import argv
from main import run
from models import NodeDefinition

if __name__ == "__main__":
    node_definition_path = argv[1]
    with open(node_definition_path, "r") as fh:
        node_definition = NodeDefinition(**json.loads(fh.read()))
    run(node_definition)
