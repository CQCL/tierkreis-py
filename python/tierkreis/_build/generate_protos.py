import glob
import os
from pathlib import Path
import subprocess

from setuptools import build_meta as default_backend  # type : ignore

# PEP 517 / 518 local (aka in-tree) backend for pip/build/etc.
# We expect to be run from the project root, i.e. python/
tierkreis_python_dir = Path(__file__).parent.parent.parent
PROTO_INPUT_DIR = str(tierkreis_python_dir / "protos")
PROTO_OUTPUT_DIR = str(tierkreis_python_dir / "tierkreis/core/protos")


def generate_proto_code():
    proto_files = glob.glob(PROTO_INPUT_DIR + "/**/*.proto", recursive=True)
    assert len(proto_files) > 0

    if not os.path.exists(PROTO_OUTPUT_DIR):
        os.mkdir(PROTO_OUTPUT_DIR)

    subprocess.check_call(
        [
            "protoc",
            "--python_betterproto_out=" + PROTO_OUTPUT_DIR,
            "--experimental_allow_proto3_optional",
            "-I",
            PROTO_INPUT_DIR,
        ]
        + proto_files
    )


def build_sdist(*args, **kwargs):
    generate_proto_code()
    return default_backend.build_sdist(*args, **kwargs)


def build_wheel(*args, **kwargs):
    if os.path.exists(PROTO_INPUT_DIR):
        generate_proto_code()
    else:
        # Building wheel from sdist.
        # Protos should have been generated already
        # and placed into the sdist.
        proto_files_already_generated = glob.glob(
            PROTO_OUTPUT_DIR + "/tierkreis/**/*.py", recursive=True
        )
        assert len(proto_files_already_generated) > 0
        # No generation required.
    return default_backend.build_wheel(*args, **kwargs)


get_requires_for_build_sdist = default_backend.get_requires_for_build_sdist

get_requires_for_build_wheel = default_backend.get_requires_for_build_wheel

prepare_metadata_for_build_wheel = default_backend.prepare_metadata_for_build_wheel


# PEP 660 defines additional hooks to support "pip install --editable":
def build_editable(*args, **kwargs):
    generate_proto_code()
    return default_backend.build_editable(*args, **kwargs)


get_requires_for_build_editable = default_backend.get_requires_for_build_editable

prepare_metadata_for_build_editable = (
    default_backend.prepare_metadata_for_build_editable
)
