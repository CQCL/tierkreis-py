from tests.sample_graph import TierkreisGraph, nexus_polling_graph
from tierkreis.core import Labels


def _outer_loop_body():
    tg = TierkreisGraph()

    ntchem_output = tg.add_func(
        "ntchem-worker.run", molecular_input=tg.input["molecular_input"]
    )["ntchem_output"]
    circuit = tg.add_func("circuit-from-ntchem.run", ntchem_output=ntchem_output)[
        "circuit"
    ]
    distribution = tg.add_func(
        "eval", thunk=tg.add_const(nexus_polling_graph()), circuit=circuit
    )["distribution"]
    energy = tg.add_func("energy-from-distribution.run", distribution=distribution)
    e1, e2 = tg.copy_value(energy)

    tg.set_outputs(
        value=tg.add_func(
            "switch",
            pred=tg.add_func("numerical-worker.igt", a=e1, b=tg.add_const(-100000)),
            if_true=tg.add_tag(Labels.BREAK, value=e2),
            if_false=tg.add_tag(Labels.CONTINUE, value=tg.add_const(5)),
        )
    )

    return tg


def vqe_graph():
    tg = TierkreisGraph()

    tg.set_outputs(
        energy=tg.add_func(
            "loop",
            body=tg.add_const(_outer_loop_body()),
            molecular_input=tg.input["molecular_input"],
        )
    )

    return tg
