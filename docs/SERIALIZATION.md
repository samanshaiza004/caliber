# Serialization and payload boundaries

Caliber moves application-defined bytes and values across a narrow in-process
boundary. It does not define a universal command language, object model, or
serialization format. The application owns the meaning and representation of
each payload; Caliber provides bounded delivery, publication, revision, handle,
and lifetime mechanics.

This guide describes how to choose and use representations at that boundary.
It complements the [design overview](DESIGN.md), the [C ABI contract](ABI.md),
and the [ownership and lifecycle rules](OWNERSHIP.md) and
[lifecycle guide](LIFECYCLE.md).

> Caliber deliberately does not choose a serialization format. Choose the
> simplest representation appropriate for each plane, then measure.

## Keep the four planes distinct

The planes have different update and retention behavior. A single application
may use different encodings for its control and state payloads, while bulk
resources and telemetry need not be serialized at all.

| Plane | What crosses the boundary | Caliber provides | The application provides |
| --- | --- | --- | --- |
| Control | Bounded opaque command bytes, delivered through a FIFO | Byte and queue bounds, copying on accepted dispatch, FIFO delivery | Command format, decoding, validation, authorization, and stale-command policy |
| State | One bounded opaque payload with a non-zero application schema value and Caliber revision | Complete latest-value publication, atomic replacement, revision assignment, and a leased read | Payload format, schema meaning and compatibility, validation, and the state represented |
| Resources | Immutable opaque byte payloads addressed by an ID and generation | Size/count bounds, immutable storage, generation-checked mapping, and leases | Resource format, interpretation, and when a resource can be retired |
| Telemetry | A fixed-width array of native `size_t` values | Coherent latest-value replacement, sequence metadata, and copying reads | Width configuration and the meaning, units, and interpretation of each slot |

Control and state are byte payloads in ABI v1, but they are not the same kind
of traffic: commands are ordered work, while state is a replaceable snapshot.
Resources are separately addressed immutable data. Telemetry is a native
latest-value slot, not a byte-encoded event history. The optional ordered SPSC
stream in `caliber-core` remains an internal Rust facility and is not exposed
by the v0.1 foreign ABI.

## Choose a representation at the application boundary

Choose a format that fits the application's actual consumers, compatibility
needs, and payload sizes. Caliber neither requires nor recommends a particular
codec. JSON, a compact binary encoding, a hand-written representation, or a
language-native format can be appropriate in an application-specific
contract; the choice does not become a Caliber feature or ABI promise.

For a small control message, a tiny fixed record can be enough. JSON is useful
when inspectability or a changing prototype matters; MessagePack or another
compact encoding may suit an application that wants a compact payload while
keeping an application-owned schema. Generated formats such as FlatBuffers or
Cap’n Proto can be a fit when the application benefits from their schema tools
and data-access model. None is a universal default, and Caliber does not rank
them for every workload.

Keep the codec beside the application contract that defines the payload. The
producer should validate the value and encode it before publication or
dispatch; the consumer should decode and validate before applying it. Treat
received bytes as untrusted input even in-process: enforce the configured
length limits, reject malformed or unsupported payload versions, and leave
application state unchanged when decoding or validation fails. Caliber can
enforce its byte and capacity limits, but it cannot validate application
meaning.

Application payload versioning is separate from Caliber ABI versioning. In
ABI v1, state publications carry a non-zero application-defined `schema`
number alongside the Caliber-assigned monotonic revision. Caliber only checks
that this value is non-zero; it does not interpret the schema or payload.
Commands and resource payloads have no Caliber-defined schema field, so any
format identification or compatibility rules for them belong to the
application contract. Do not treat the ABI version as the version of an
application payload.

## Use each plane for its intended update pattern

### Control: ordered semantic work

Dispatch copies the command bytes before returning. Accepted packets are
queued in FIFO order within the configured count and byte bounds. A full queue
or an oversized packet is reported; Caliber does not decode the packet,
coalesce commands, or choose what a stale revision means. The application
interprets the command and owns its domain policy.

Keep control messages coarse-grained. A semantic operation such as “select
document” is a good boundary unit; repeated property reads or one command per
keystroke turn the boundary into a request-per-property protocol. High-rate
pointer, caret, hover, layout, and paint updates belong in the frontend.

### State: coherent latest snapshot

Publish a complete bounded payload when the application has a new coherent
state. A successful publish gets the next revision and replaces the latest
publication atomically. A failed publish leaves the previous publication
visible. Readers lease one immutable snapshot and release the lease exactly
once; the lease lifetime is defined in the [ABI and lifecycle docs](ABI.md).

State should summarize what a frontend needs to present and refer to large
data separately. It should not routinely contain a whole document, image, or
other bulk payload just because the selected codec can encode it. Nor should a
new snapshot be serialized solely to repaint an unchanged frame. Publish when
application-visible state changes, then let the frontend render locally.

JSON can be a reasonable state representation while the schema is changing
and snapshots remain small. Consider a binary or generated schema only after
serialization/copying is measurable in the consuming application or stronger
schema-evolution guarantees justify the added tooling and maintenance.

### Resources: immutable bulk data

Publish a bounded immutable byte payload when consumers need a larger object
that does not fit naturally in a small state snapshot. Caliber assigns an ID
and generation; consumers map that exact handle, read the bytes while holding
the lease, and release the view. Retiring the handle prevents later maps, but
already mapped views remain valid until their leases are released.

The application defines the bytes' format and how consumers know what they
mean. Include any necessary resource reference or format identification in
the application contract; Caliber does not add resource schemas. A mapped
resource is not a new serialization on each read or repaint. Publish only
when the immutable content changes, and retire it when the application no
longer needs new mappings.

### Telemetry: newest coherent sample

Configure a fixed width, then publish exactly that many `size_t` values. A
successful publication replaces the previous sample, increments its
sequence, and retains no history. Readers receive a coherent copy into
caller-owned storage. Intermediate values may be overwritten by design; use
an ordered stream only if preserving samples is required and a suitable stream
contract exists.

ABI v1 telemetry is a same-process, same-pointer-width native representation,
not a portable wire format. Its `schema` and `reserved` metadata fields are
zero; the application defines each slot's meaning. Caliber does not promise
that its foreign calls are realtime-safe: the ABI publication also advances
the wake signal and may involve synchronization. See [ABI compatibility and
telemetry details](ABI.md#data-plane-v01-decisions).

## Keep work bounded and failures explicit

Set context limits to fit the application's legitimate maximum payload and
queue needs. ABI v1 exposes limits for command bytes, state-publication bytes,
resource bytes, resource count, telemetry width, and pending command count.
The library also enforces hard maxima. A caller must handle too-large,
queue-full, unavailable, stale, and unsupported-version results rather than
assuming every submission succeeds. Check the exact status and ownership
rules in [`include/caliber.h`](../include/caliber.h) and [ABI.md](ABI.md).

Serialization cost belongs to the application codec; Caliber adds boundary
costs such as copying command and state bytes, retaining immutable resource
storage, and copying telemetry samples to readers. Measure encoding,
decoding, copying, publication, and consumption separately when these costs
matter. Avoid repeating serialization or whole-payload copies on a render,
audio, or other high-frequency path. Generic Caliber ABI calls may allocate
or lock and are not suitable for hard realtime callbacks.

## The ABI is not a network protocol

The C ABI is an in-process native interface. Its records use native pointers
and `size_t`; consumers must match the library's process architecture and
pointer width. Payload bytes may use an application-chosen portable encoding,
but that does not make Caliber ABI v1 a cross-process or cross-architecture
wire protocol. Caliber currently defines no network framing, authentication,
reconnection, or transport-level delivery guarantees.

If an application later needs another transport, specify and validate that
transport boundary in the application that owns the payload semantics. Keep
transport framing and application payload representation explicit; do not
infer a generic serialization or IPC framework from Caliber's opaque byte
fields.

## Practical review questions

The [canonical C lifecycle example](../examples/c-counter/README.md) shows
application-owned command and state bytes alongside an immutable resource. The
[Python `ctypes` example](../examples/python-ctypes/README.md) demonstrates the
same opaque-byte contract from another language; its hand-mirrored declarations
are example code, not a Caliber SDK.

Before adding or changing a payload, ask:

1. Is it ordered work, replaceable state, immutable bulk data, or a latest
   sample? Does the selected plane match that behavior?
2. Which application component owns the format, version, validation, and
   stale-data policy?
3. Are size, count, lifetime, and failure behavior bounded and documented?
4. Can a frontend update presentation locally without resending the payload?
5. Does the representation preserve the application's required data
   fidelity, and can the consumer reject malformed or unsupported input?

If the same generic schema, codec, or message framework appears necessary for
unrelated applications, first check whether the applications actually share
the same semantics. Caliber's boundary is intentionally format-agnostic; a
universal payload framework is outside its current scope.
