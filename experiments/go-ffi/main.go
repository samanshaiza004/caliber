// Command go-ffi-smoke is a deliberately small foreign-frontend smoke test.
//
// It exercises only the public C function table. There is no GUI, Go-side
// model, or Caliber-specific serialization here: commands are opaque bytes
// and the read paths expose only the ABI's status/lease behavior.
package main

/*
#cgo darwin LDFLAGS: -lcaliber_ffi
#cgo linux LDFLAGS: -lcaliber_ffi
#include <stddef.h>
#include <stdint.h>

typedef struct CaliberContext CaliberContext;

typedef int32_t CaliberStatus;
enum {
	CaliberOk = 0,
	CaliberInvalidArgument = 1,
	CaliberInvalidHandle = 2,
	CaliberBufferTooSmall = 3,
	CaliberLimitExceeded = 4,
	CaliberNotFound = 5,
	CaliberStale = 6,
	CaliberUnavailable = 7,
	CaliberQueueFull = 8,
	CaliberUnsupportedVersion = 9,
	CaliberInternal = 10
};

typedef struct {
	uint32_t struct_size;
	size_t max_command_bytes;
	size_t max_publication_bytes;
	size_t max_resource_bytes;
	size_t max_resources;
	size_t telemetry_width;
	size_t max_pending_commands;
} CaliberContextConfig;

typedef struct {
	uint64_t revision;
	uint32_t schema;
	uint32_t reserved;
	const uint8_t *data;
	size_t len;
	void *lease;
} CaliberStatePublication;

typedef struct {
	uint64_t resource_id;
	uint64_t generation;
	const uint8_t *data;
	size_t len;
	void *lease;
} CaliberResourceView;

typedef struct {
	uint64_t sequence;
	uint32_t schema;
	uint32_t reserved;
	size_t value_count;
	size_t value_size;
} CaliberTelemetryInfo;

typedef struct CaliberApiV1 CaliberApiV1;
typedef CaliberStatus (*context_create_fn)(const CaliberContextConfig *, CaliberContext **);
typedef void (*context_destroy_fn)(CaliberContext *);
typedef CaliberStatus (*context_dispatch_fn)(const CaliberContext *, const uint8_t *, size_t);
typedef CaliberStatus (*context_peek_command_fn)(const CaliberContext *, size_t *);
typedef CaliberStatus (*context_take_command_fn)(const CaliberContext *, uint8_t *, size_t, size_t *);
typedef CaliberStatus (*context_publish_state_fn)(const CaliberContext *, uint32_t, const uint8_t *, size_t, uint64_t *);
typedef CaliberStatus (*context_read_state_fn)(const CaliberContext *, CaliberStatePublication *);
typedef void (*state_release_fn)(CaliberStatePublication *);
typedef CaliberStatus (*context_map_resource_fn)(const CaliberContext *, uint64_t, uint64_t, CaliberResourceView *);
typedef void (*resource_release_fn)(CaliberResourceView *);
typedef CaliberStatus (*context_publish_resource_fn)(const CaliberContext *, const uint8_t *, size_t, uint64_t *, uint64_t *);
typedef CaliberStatus (*context_release_resource_fn)(const CaliberContext *, uint64_t, uint64_t);
typedef CaliberStatus (*context_publish_telemetry_fn)(const CaliberContext *, const size_t *, size_t);
typedef CaliberStatus (*context_read_telemetry_fn)(const CaliberContext *, size_t *, size_t, CaliberTelemetryInfo *);
typedef CaliberStatus (*context_wake_sequence_fn)(const CaliberContext *, uint64_t *);

struct CaliberApiV1 {
	uint32_t abi_version;
	uint32_t struct_size;
	context_create_fn context_create;
	context_destroy_fn context_destroy;
	context_dispatch_fn context_dispatch;
	context_peek_command_fn context_peek_command;
	context_take_command_fn context_take_command;
	context_publish_state_fn context_publish_state;
	context_read_state_fn context_read_latest_state;
	state_release_fn state_publication_release;
	context_map_resource_fn context_map_resource;
	resource_release_fn resource_release;
	context_publish_resource_fn context_publish_resource;
	context_release_resource_fn context_release_resource;
	context_publish_telemetry_fn context_publish_telemetry;
	context_read_telemetry_fn context_read_latest_telemetry;
	context_wake_sequence_fn context_wake_sequence;
};

extern const CaliberApiV1 *caliber_get_api(uint32_t version);

static inline const CaliberApiV1 *go_get_api(uint32_t version) {
	return caliber_get_api(version);
}

static inline CaliberStatus go_create(const CaliberApiV1 *api,
	const CaliberContextConfig *config, CaliberContext **out) {
	return api->context_create(config, out);
}

static inline void go_destroy(const CaliberApiV1 *api, CaliberContext *context) {
	api->context_destroy(context);
}

static inline CaliberStatus go_dispatch(const CaliberApiV1 *api,
	CaliberContext *context, const uint8_t *bytes, size_t len) {
	return api->context_dispatch(context, bytes, len);
}

static inline CaliberStatus go_peek_command(const CaliberApiV1 *api,
	const CaliberContext *context, size_t *out_len) {
	return api->context_peek_command(context, out_len);
}

static inline CaliberStatus go_take_command(const CaliberApiV1 *api,
	const CaliberContext *context, uint8_t *out, size_t capacity, size_t *out_len) {
	return api->context_take_command(context, out, capacity, out_len);
}

static inline CaliberStatus go_publish_state(const CaliberApiV1 *api,
	const CaliberContext *context, uint32_t schema, const uint8_t *bytes,
	size_t len, uint64_t *revision) {
	return api->context_publish_state(context, schema, bytes, len, revision);
}

static inline CaliberStatus go_read_state(const CaliberApiV1 *api,
	const CaliberContext *context, CaliberStatePublication *out) {
	return api->context_read_latest_state(context, out);
}

static inline void go_release_state(const CaliberApiV1 *api,
	CaliberStatePublication *publication) {
	api->state_publication_release(publication);
}

static inline CaliberStatus go_map_resource(const CaliberApiV1 *api,
	const CaliberContext *context, uint64_t id, uint64_t generation,
	CaliberResourceView *out) {
	return api->context_map_resource(context, id, generation, out);
}

static inline void go_release_resource(const CaliberApiV1 *api,
	CaliberResourceView *view) {
	api->resource_release(view);
}

static inline CaliberStatus go_publish_resource(const CaliberApiV1 *api,
	const CaliberContext *context, const uint8_t *bytes, size_t len,
	uint64_t *resource_id, uint64_t *generation) {
	return api->context_publish_resource(context, bytes, len, resource_id, generation);
}

static inline CaliberStatus go_release_resource_owner(const CaliberApiV1 *api,
	const CaliberContext *context, uint64_t resource_id, uint64_t generation) {
	return api->context_release_resource(context, resource_id, generation);
}

static inline CaliberStatus go_publish_telemetry(const CaliberApiV1 *api,
	const CaliberContext *context, const size_t *values, size_t count) {
	return api->context_publish_telemetry(context, values, count);
}

static inline CaliberStatus go_read_telemetry(const CaliberApiV1 *api,
	const CaliberContext *context, size_t *out, size_t capacity,
	CaliberTelemetryInfo *info) {
	return api->context_read_latest_telemetry(context, out, capacity, info);
}

static inline CaliberStatus go_wake_sequence(const CaliberApiV1 *api,
	const CaliberContext *context, uint64_t *out) {
	return api->context_wake_sequence(context, out);
}
*/
import "C"

import (
	"fmt"
	"unsafe"
)

const abiVersion = 1

const (
	statusOK              C.CaliberStatus = C.CaliberOk
	statusInvalidArgument C.CaliberStatus = C.CaliberInvalidArgument
	statusInvalidHandle   C.CaliberStatus = C.CaliberInvalidHandle
	statusBufferTooSmall  C.CaliberStatus = C.CaliberBufferTooSmall
	statusLimitExceeded   C.CaliberStatus = C.CaliberLimitExceeded
	statusNotFound        C.CaliberStatus = C.CaliberNotFound
	statusStale           C.CaliberStatus = C.CaliberStale
	statusUnavailable     C.CaliberStatus = C.CaliberUnavailable
)

func statusName(status C.CaliberStatus) string {
	switch status {
	case statusOK:
		return "ok"
	case statusInvalidArgument:
		return "invalid-argument"
	case statusInvalidHandle:
		return "invalid-handle"
	case statusBufferTooSmall:
		return "buffer-too-small"
	case statusLimitExceeded:
		return "limit-exceeded"
	case statusNotFound:
		return "not-found"
	case statusUnavailable:
		return "unavailable"
	default:
		return fmt.Sprintf("status-%d", int32(status))
	}
}

func requireStatus(label string, got, want C.CaliberStatus) {
	if got != want {
		panic(fmt.Sprintf("%s: got %s, want %s", label, statusName(got), statusName(want)))
	}
}

func dispatch(api *C.CaliberApiV1, context *C.CaliberContext, payload []byte) C.CaliberStatus {
	if len(payload) == 0 {
		return C.go_dispatch(api, context, nil, 0)
	}
	return C.go_dispatch(api, context,
		(*C.uint8_t)(unsafe.Pointer(&payload[0])), C.size_t(len(payload)))
}

func main() {
	if api := C.go_get_api(abiVersion + 1); api != nil {
		panic("unsupported ABI version unexpectedly returned a table")
	}
	api := C.go_get_api(abiVersion)
	if api == nil {
		panic("ABI v1 is unavailable")
	}
	if api.abi_version != abiVersion || api.struct_size == 0 {
		panic(fmt.Sprintf("invalid API table: version=%d size=%d", api.abi_version, api.struct_size))
	}

	config := C.CaliberContextConfig{
		struct_size:           C.uint32_t(C.sizeof_CaliberContextConfig),
		max_command_bytes:     4,
		max_publication_bytes: 16,
		max_resource_bytes:    16,
		max_resources:         4,
		telemetry_width:       2,
		max_pending_commands:  2,
	}
	var context *C.CaliberContext
	requireStatus("create", C.go_create(api, &config, &context), statusOK)
	if context == nil {
		panic("create returned a nil context")
	}
	defer C.go_destroy(api, context)

	var wakeBefore C.uint64_t
	requireStatus("initial wake read", C.go_wake_sequence(api, context, &wakeBefore), statusOK)
	requireStatus("null context", C.go_dispatch(api, nil, nil, 0), statusInvalidHandle)
	requireStatus("null non-empty command", C.go_dispatch(api, context, nil, 1), statusInvalidArgument)
	requireStatus("empty command", dispatch(api, context, nil), statusOK)
	requireStatus("opaque command", dispatch(api, context, []byte("ping")), statusOK)
	var commandLen C.size_t
	requireStatus("peek command", C.go_peek_command(api, context, &commandLen), statusOK)
	if commandLen != 0 {
		panic(fmt.Sprintf("unexpected command length: %d", commandLen))
	}
	requireStatus("take empty command", C.go_take_command(api, context, nil, 0, &commandLen), statusOK)
	requireStatus("peek opaque command", C.go_peek_command(api, context, &commandLen), statusOK)
	if commandLen != 4 {
		panic(fmt.Sprintf("unexpected opaque command length: %d", commandLen))
	}
	shortCommand := make([]C.uint8_t, 1)
	requireStatus("short command buffer", C.go_take_command(api, context,
		(*C.uint8_t)(unsafe.Pointer(&shortCommand[0])), 1, &commandLen), statusBufferTooSmall)
	command := make([]C.uint8_t, 4)
	requireStatus("take command", C.go_take_command(api, context,
		(*C.uint8_t)(unsafe.Pointer(&command[0])), C.size_t(len(command)), &commandLen), statusOK)
	if string(C.GoBytes(unsafe.Pointer(&command[0]), C.int(commandLen))) != "ping" {
		panic("command bytes were not preserved")
	}
	var wakeAfter C.uint64_t
	requireStatus("wake after command", C.go_wake_sequence(api, context, &wakeAfter), statusOK)
	if wakeAfter <= wakeBefore {
		panic(fmt.Sprintf("wake sequence did not advance: before=%d after=%d", wakeBefore, wakeAfter))
	}
	wakeBefore = wakeAfter
	requireStatus("oversized command", dispatch(api, context, []byte("12345")), statusLimitExceeded)
	requireStatus("wake after rejection", C.go_wake_sequence(api, context, &wakeAfter), statusOK)
	if wakeAfter != wakeBefore {
		panic(fmt.Sprintf("rejected command changed wake sequence: before=%d after=%d", wakeBefore, wakeAfter))
	}

	// Before a producer publishes, the read path reports unavailable. Releasing
	// that zero view must remain harmless.
	var publication C.CaliberStatePublication
	requireStatus("empty state", C.go_read_state(api, context, &publication), statusUnavailable)
	C.go_release_state(api, &publication)
	if publication.lease != nil || publication.data != nil {
		panic("state release did not clear the empty view")
	}

	var revision C.uint64_t
	requireStatus("null state payload", C.go_publish_state(api, context, 7, nil, 1, &revision), statusInvalidArgument)
	stateBytes := []byte("state")
	requireStatus("state publish", C.go_publish_state(api, context, 7,
		(*C.uint8_t)(unsafe.Pointer(&stateBytes[0])), C.size_t(len(stateBytes)), &revision), statusOK)
	if revision != 1 {
		panic(fmt.Sprintf("first state revision was %d, want 1", revision))
	}
	requireStatus("published state read", C.go_read_state(api, context, &publication), statusOK)
	if publication.revision != revision || publication.schema != 7 || publication.len != 5 {
		panic(fmt.Sprintf("unexpected state view: revision=%d schema=%d len=%d", publication.revision, publication.schema, publication.len))
	}
	if got := C.GoBytes(unsafe.Pointer(publication.data), C.int(publication.len)); string(got) != "state" {
		panic(fmt.Sprintf("unexpected state payload %q", got))
	}
	C.go_release_state(api, &publication)
	if publication.lease != nil || publication.data != nil {
		panic("state release did not clear the published view")
	}

	var resource C.CaliberResourceView
	requireStatus("unknown resource", C.go_map_resource(api, context, 99, 1, &resource), statusNotFound)
	C.go_release_resource(api, &resource)
	if resource.lease != nil || resource.data != nil {
		panic("resource release did not clear the empty view")
	}

	var resourceID, generation C.uint64_t
	requireStatus("null resource payload", C.go_publish_resource(api, context, nil, 1, &resourceID, &generation), statusInvalidArgument)
	resourceBytes := []byte("waveform")
	requireStatus("resource publish", C.go_publish_resource(api, context,
		(*C.uint8_t)(unsafe.Pointer(&resourceBytes[0])), C.size_t(len(resourceBytes)),
		&resourceID, &generation), statusOK)
	requireStatus("resource map", C.go_map_resource(api, context, resourceID, generation, &resource), statusOK)
	if resource.len != C.size_t(len(resourceBytes)) {
		panic(fmt.Sprintf("unexpected resource length %d", resource.len))
	}
	if got := C.GoBytes(unsafe.Pointer(resource.data), C.int(resource.len)); string(got) != string(resourceBytes) {
		panic(fmt.Sprintf("unexpected resource payload %q", got))
	}
	requireStatus("resource owner release", C.go_release_resource_owner(api, context, resourceID, generation), statusOK)
	if got := C.GoBytes(unsafe.Pointer(resource.data), C.int(resource.len)); string(got) != string(resourceBytes) {
		panic(fmt.Sprintf("mapped lease changed after owner release: %q", got))
	}
	C.go_release_resource(api, &resource)
	fmt.Printf("resource owner released: id=%d generation=%d\n", resourceID, generation)
	requireStatus("released resource map", C.go_map_resource(api, context, resourceID, generation, &resource), statusStale)

	telemetry := make([]C.size_t, 2)
	var info C.CaliberTelemetryInfo
	requireStatus("null telemetry payload", C.go_publish_telemetry(api, context, nil, 1), statusInvalidArgument)
	values := []C.size_t{44, 8}
	requireStatus("telemetry publish", C.go_publish_telemetry(api, context,
		(*C.size_t)(unsafe.Pointer(&values[0])), C.size_t(len(values))), statusOK)
	requireStatus("wrong telemetry capacity", C.go_read_telemetry(api, context,
		(*C.size_t)(unsafe.Pointer(&telemetry[0])), 1, &info), statusBufferTooSmall)
	requireStatus("published telemetry", C.go_read_telemetry(api, context,
		(*C.size_t)(unsafe.Pointer(&telemetry[0])), C.size_t(len(telemetry)), &info), statusOK)
	if info.sequence == 0 || info.value_count != 2 || telemetry[0] != 44 || telemetry[1] != 8 {
		panic(fmt.Sprintf("unexpected telemetry: sequence=%d count=%d values=%v", info.sequence, info.value_count, telemetry))
	}
	requireStatus("null telemetry info", C.go_read_telemetry(api, context,
		(*C.size_t)(unsafe.Pointer(&telemetry[0])), C.size_t(len(telemetry)), nil), statusInvalidArgument)

	// Null release pointers are explicitly harmless and are part of the ABI's
	// cleanup contract.
	C.go_release_state(api, nil)
	C.go_release_resource(api, nil)

	fmt.Println("caliber Go FFI smoke: PASS")
	fmt.Printf("ABI v%d, wake sequence %d, command bytes bounded at %d\n", api.abi_version, wakeAfter, config.max_command_bytes)
}
