"""Small Caliber design fixture; stdlib only and intentionally non-production."""

from dataclasses import dataclass
from typing import Optional


class TraceError(Exception):
    pass


@dataclass(frozen=True)
class ResourceRef:
    resource_id: int
    generation: int
    length: int


@dataclass(frozen=True)
class State:
    revision: int
    query: str
    selected: Optional[int]
    waveform: Optional[ResourceRef]


class SyntheticBackend:
    """Model control/state/data ownership without pretending to be core."""

    def __init__(self) -> None:
        self.samples = {1: "kick", 2: "snare", 3: "hat"}
        self.state = State(0, "", None, None)
        self.resources = {(7, 1): bytes(range(32))}
        self.meter_sequence = 0
        self.latest_meter = (0, 0.0)
        self.overwritten_meter_values = 0

    def dispatch(self, command: dict, based_on_revision: int) -> State:
        if not isinstance(command, dict) or command.get("kind") not in {
            "search",
            "select",
            "preview",
        }:
            raise TraceError("unknown command")

        # Staleness is deliberately an application policy decision. This
        # fixture accepts commands and records the resulting next revision.
        query = self.state.query
        selected = self.state.selected
        waveform = self.state.waveform
        kind = command["kind"]
        if kind == "search":
            query = command["query"]
        elif kind == "select":
            selected = command["sample_id"]
            waveform = ResourceRef(7, 1, len(self.resources[(7, 1)]))
        elif kind == "preview" and command["sample_id"] not in self.samples:
            raise TraceError("unknown sample")
        if not isinstance(based_on_revision, int):
            raise TraceError("invalid based_on_revision")

        self.state = State(self.state.revision + 1, query, selected, waveform)
        return self.state

    def try_publish(self, candidate: State) -> bool:
        if candidate.revision <= self.state.revision:
            return False
        if candidate.waveform is not None:
            ref = candidate.waveform
            if (ref.resource_id, ref.generation) not in self.resources:
                return False
            if ref.length != len(self.resources[(ref.resource_id, ref.generation)]):
                return False
        self.state = candidate
        return True

    def map_resource(self, ref: ResourceRef) -> bytes:
        try:
            data = self.resources[(ref.resource_id, ref.generation)]
        except KeyError as exc:
            raise TraceError("expired resource") from exc
        if ref.length != len(data):
            raise TraceError("resource length mismatch")
        return data

    def publish_meter(self, value: float) -> None:
        if self.latest_meter[0] != 0:
            self.overwritten_meter_values += 1
        self.meter_sequence += 1
        self.latest_meter = (self.meter_sequence, value)


def run_trace() -> None:
    backend = SyntheticBackend()
    initial = backend.state
    backend.dispatch({"kind": "search", "query": "snare"}, initial.revision)
    selected = backend.dispatch({"kind": "select", "sample_id": 2}, backend.state.revision)
    assert selected.waveform is not None
    assert backend.map_resource(selected.waveform) == bytes(range(32))

    # A malformed next publication cannot replace the last valid one.
    malformed = State(selected.revision + 1, "bad", None, ResourceRef(7, 99, 32))
    assert not backend.try_publish(malformed)
    assert backend.state == selected

    # The latest-value plane drops intermediate history but stays coherent.
    for value in range(10):
        backend.publish_meter(value / 10)
    sequence, value = backend.latest_meter
    assert sequence == 10 and value == 0.9
    assert backend.overwritten_meter_values == 9

    # A stale generation is rejected instead of aliasing a later resource.
    try:
        backend.map_resource(ResourceRef(7, 2, 32))
    except TraceError as error:
        assert str(error) == "expired resource"
    else:
        raise AssertionError("expired resource was accepted")

    print("synthetic trace: PASS")
    print(f"state revision: {backend.state.revision}")
    print(f"meter overwrites: {backend.overwritten_meter_values}")
    print("direct integration comparison: required before any extraction")


if __name__ == "__main__":
    run_trace()
