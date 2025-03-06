import pytest

from tierkreis.controller.models import NodeLocation

node_location_1 = NodeLocation(location=[])
node_location_1 = node_location_1.append_node(0)
node_location_1 = node_location_1.append_loop(0)
node_location_1 = node_location_1.append_node(3)
node_location_1 = node_location_1.append_loop(2)
node_location_1 = node_location_1.append_node(0)
node_location_1 = node_location_1.append_map(8)
node_location_1 = node_location_1.append_node(0)


node_location_2 = NodeLocation(location=[])
node_location_2 = node_location_2.append_node(0)
node_location_2 = node_location_2.append_loop(0)
node_location_2 = node_location_2.append_node(3)
node_location_2 = node_location_2.append_node(8)
node_location_2 = node_location_2.append_node(0)

node_location_3 = NodeLocation(location=[])
node_location_3 = node_location_3.append_node(0)

node_location_4 = NodeLocation(location=[])


@pytest.mark.parametrize(
    ["node_location", "loc_str"],
    [
        (node_location_1, "N0.L0.N3.L2.N0.M8.N0"),
        (node_location_2, "N0.L0.N3.N8.N0"),
        (node_location_3, "N0"),
        (node_location_4, ""),
    ],
)
def test_to_str(node_location: NodeLocation, loc_str: str):
    assert str(node_location) == loc_str
