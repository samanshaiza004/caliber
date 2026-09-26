/*
 * Canonical Caliber C ABI v1 declaration.
 *
 * This header is the public ABI source of truth. The Rust implementation is
 * checked against its layouts by crates/caliber-abi-tests on all CI hosts.
 * Do not edit a consumer-local copy; include this file from the Caliber source
 * dependency. The first v0.1 release is intended to promise an append-only
 * ABI v1; this pre-v0.1 candidate may still evolve. Once v0.1 is released,
 * existing status values and field order/meaning are frozen. An incompatible
 * change requires ABI v2.
 */
#ifndef CALIBER_H
#define CALIBER_H

#include <stddef.h>
#include <stdint.h>

#define CALIBER_ABI_VERSION_1 UINT32_C(1)

#ifdef __cplusplus
extern "C" {
#endif

/* ABI v1 result numbers; do not renumber or reuse them after the v0.1 release.
 * The C type is explicitly fixed-width to match Rust's repr(i32).
 */
typedef int32_t CaliberStatus;
enum {
    CALIBER_STATUS_OK = 0,
    CALIBER_STATUS_INVALID_ARGUMENT = 1,
    CALIBER_STATUS_INVALID_HANDLE = 2,
    CALIBER_STATUS_BUFFER_TOO_SMALL = 3,
    CALIBER_STATUS_LIMIT_EXCEEDED = 4,
    CALIBER_STATUS_NOT_FOUND = 5,
    CALIBER_STATUS_STALE = 6,
    CALIBER_STATUS_UNAVAILABLE = 7,
    CALIBER_STATUS_QUEUE_FULL = 8,
    CALIBER_STATUS_UNSUPPORTED_VERSION = 9,
    CALIBER_STATUS_INTERNAL = 10,
    CALIBER_STATUS_STOPPED = 11
};

/* Opaque context. The caller creates/destroys it through CaliberApiV1. */
typedef struct CaliberContext CaliberContext;

/*
 * Caller limits for a new context. struct_size is the number of bytes the
 * caller supplies, including this field. Older prefixes are accepted; fields
 * not fully present use library defaults. A zero struct_size selects defaults.
 * size_t fields require a caller and library with the same pointer width.
 */
typedef struct CaliberContextConfig {
    uint32_t struct_size;
    size_t max_command_bytes;
    size_t max_publication_bytes;
    size_t max_resource_bytes;
    size_t max_resources;
    size_t telemetry_width;
    size_t max_pending_commands;
} CaliberContextConfig;

/*
 * Immutable latest-state view. data/len are borrowed under lease and remain
 * valid until state_publication_release is called. The lease may outlive its
 * originating context. The caller must release each successful read once.
 */
typedef struct CaliberStatePublication {
    uint64_t revision;
    uint32_t schema;
    uint32_t reserved;
    const uint8_t *data;
    size_t len;
    void *lease;
} CaliberStatePublication;

/*
 * Immutable resource view. data/len are borrowed under lease and remain valid
 * until resource_release is called. The lease may outlive its context. This
 * view release is separate from context_release_resource, which retires the
 * resource handle from the context.
 */
typedef struct CaliberResourceView {
    uint64_t resource_id;
    uint64_t generation;
    const uint8_t *data;
    size_t len;
    void *lease;
} CaliberResourceView;

/*
 * Latest-value telemetry metadata. ABI v1 samples contain exactly
 * CaliberContextConfig.telemetry_width native size_t values. value_size is
 * sizeof(size_t); schema and reserved are zero in ABI v1. This is a
 * same-process, same-pointer-width representation, not a wire format.
 * Telemetry is copied into caller-owned storage; this record is not a lease.
 */
typedef struct CaliberTelemetryInfo {
    uint64_t sequence;
    uint32_t schema;
    uint32_t reserved;
    size_t value_count;
    size_t value_size;
} CaliberTelemetryInfo;

/*
 * Versioned function table returned by caliber_get_api. The table is owned by
 * Caliber and remains valid for the lifetime of the loaded library. Check
 * abi_version and struct_size before reading fields. ABI v1 extensions append
 * entries only; callers must check that struct_size reaches a field before
 * reading or calling it. Function-pointer entries may be null when a shorter
 * compatible table omits a trailing operation.
 */
typedef struct CaliberApiV1 {
    uint32_t abi_version;
    uint32_t struct_size;
    /* Creates one Caliber-owned context; NULL config selects defaults. */
    CaliberStatus (*context_create)(const CaliberContextConfig *, CaliberContext **);
    /* Requires all calls and frontend-owned waiter threads to have stopped. */
    void (*context_destroy)(CaliberContext *);
    /* Copies command bytes before returning; nonzero len requires data. */
    CaliberStatus (*context_dispatch)(const CaliberContext *, const uint8_t *, size_t);
    CaliberStatus (*context_peek_command)(const CaliberContext *, size_t *);
    CaliberStatus (*context_take_command)(const CaliberContext *, uint8_t *, size_t, size_t *);
    CaliberStatus (*context_publish_state)(const CaliberContext *, uint32_t, const uint8_t *, size_t, uint64_t *);
    /* A successful read creates one lease; release it with the next entry. */
    CaliberStatus (*context_read_latest_state)(const CaliberContext *, CaliberStatePublication *);
    void (*state_publication_release)(CaliberStatePublication *);
    /* A successful map creates one lease; release it with resource_release. */
    CaliberStatus (*context_map_resource)(const CaliberContext *, uint64_t, uint64_t, CaliberResourceView *);
    void (*resource_release)(CaliberResourceView *);
    CaliberStatus (*context_publish_resource)(const CaliberContext *, const uint8_t *, size_t, uint64_t *, uint64_t *);
    CaliberStatus (*context_release_resource)(const CaliberContext *, uint64_t, uint64_t);
    /* Replaces the whole fixed-width latest sample; does not retain history. */
    CaliberStatus (*context_publish_telemetry)(const CaliberContext *, const size_t *, size_t);
    /* Copies telemetry into caller storage; info is caller-owned metadata. */
    CaliberStatus (*context_read_latest_telemetry)(const CaliberContext *, size_t *, size_t, CaliberTelemetryInfo *);
    CaliberStatus (*context_wake_sequence)(const CaliberContext *, uint64_t *);
    /* Blocks until the sequence changes or waiters are permanently stopped. */
    CaliberStatus (*context_wait_wake)(const CaliberContext *, uint64_t, uint64_t *);
    /* Permanently stops and wakes waiters; caller owns and joins its thread. */
    CaliberStatus (*context_stop_wake_waiters)(const CaliberContext *);
} CaliberApiV1;

/* Returns the table for version 1, or NULL for an unsupported version. */
const CaliberApiV1 *caliber_get_api(uint32_t requested_version);

#ifdef __cplusplus
}
#endif

#endif /* CALIBER_H */
