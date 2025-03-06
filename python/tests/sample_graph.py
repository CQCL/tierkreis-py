from tierkreis.core import Labels, TierkreisGraph


def _loop_graph() -> TierkreisGraph:
    ifg = TierkreisGraph()
    ifg.set_outputs(value=ifg.add_tag(Labels.BREAK, value=ifg.input["x"]))

    elg = TierkreisGraph()
    elg.set_outputs(
        value=elg.add_tag(
            Labels.CONTINUE,
            value=elg.add_func("iadd", a=elg.input["x"], b=elg.add_const(1)),
        )
    )

    tg = TierkreisGraph()
    v1, v2 = tg.copy_value(tg.input["value"])
    tg.set_outputs(
        value=tg.add_func(
            "eval",
            thunk=tg.add_func(
                "switch",
                pred=tg.add_func("igt", a=v1, b=tg.add_const(5)),
                if_true=tg.add_const(ifg),
                if_false=tg.add_const(elg),
            ),
            x=v2,
        )["value"]
    )
    return tg


def sample_graph() -> TierkreisGraph:
    one_graph = TierkreisGraph()
    one_graph.set_outputs(
        value=one_graph.add_func(
            "iadd", a=one_graph.input["value"], b=one_graph.input["other"]
        )
    )
    many_graph = TierkreisGraph()
    many_graph.discard(many_graph.input["other"])
    many_graph.set_outputs(
        value=many_graph.add_func("id", value=many_graph.input["value"])
    )

    tg = TierkreisGraph()
    tg.set_outputs(
        out=tg.input["inp"],
        b=tg.add_func("iadd", a=tg.add_const(1), b=tg.add_const(3)),
        tag=tg.add_tag("boo", value=tg.add_const("world")),
        add=tg.add_func(
            "python_nodes::python_add", a=tg.add_const(23), b=tg.add_const(123)
        ),
        _and=tg.add_func("and", a=tg.add_const(True), b=tg.add_const(False)),
        result=tg.add_func(
            "eval",
            thunk=tg.add_match(
                tg.input["vv"],
                one=tg.add_const(one_graph),
                many=tg.add_const(many_graph),
            )["thunk"],
            other=tg.add_const(2),
        ),
        loop_out=tg.add_func(
            "loop", body=tg.add_const(_loop_graph()), value=tg.add_const(2)
        )["value"],
    )
    return tg


def sample_graph_without_match() -> TierkreisGraph:
    def loop_graph() -> TierkreisGraph:
        ifg = TierkreisGraph()
        ifg.set_outputs(value=ifg.add_tag(Labels.BREAK, value=ifg.input["x"]))

        elg = TierkreisGraph()
        elg.set_outputs(
            value=elg.add_tag(
                Labels.CONTINUE,
                value=elg.add_func(
                    "numerical-worker.iadd", a=elg.input["x"], b=elg.add_const(1)
                ),
            )
        )

        tg = TierkreisGraph()
        v1, v2 = tg.copy_value(tg.input["value"])
        tg.set_outputs(
            value=tg.add_func(
                "eval",
                thunk=tg.add_func(
                    "switch",
                    pred=tg.add_func("numerical-worker.igt", a=v1, b=tg.add_const(5)),
                    if_true=tg.add_const(ifg),
                    if_false=tg.add_const(elg),
                ),
                x=v2,
            )["value"]
        )
        return tg

    one_graph = TierkreisGraph()
    one_graph.set_outputs(
        value=one_graph.add_func(
            "numerical-worker.iadd",
            a=one_graph.input["value"],
            b=one_graph.input["other"],
        )
    )
    many_graph = TierkreisGraph()
    many_graph.discard(many_graph.input["other"])
    many_graph.set_outputs(
        value=many_graph.add_func(
            "numerical-worker.id", value=many_graph.input["value"]
        )
    )

    tg = TierkreisGraph()
    tg.set_outputs(
        out=tg.input["inp"],
        b=tg.add_func("numerical-worker.iadd", a=tg.add_const(1), b=tg.add_const(3)),
        tag=tg.add_tag("boo", value=tg.add_const("world")),
        add=tg.add_func(
            "numerical-worker.iadd", a=tg.add_const(23), b=tg.add_const(123)
        ),
        _and=tg.add_func(
            "numerical-worker.and", a=tg.add_const(True), b=tg.add_const(False)
        ),
        loop_out=tg.add_func(
            "loop", body=tg.add_const(loop_graph()), value=tg.add_const(2)
        )["value"],
    )
    return tg
