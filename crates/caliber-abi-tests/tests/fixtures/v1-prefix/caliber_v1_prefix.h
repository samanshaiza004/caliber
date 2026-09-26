/*
 * Frozen Caliber ABI v1 prefix from commit a5e0b829172eb3000517ac62a7c6f874c3ed1eae (before blocking wake-wait
 * entries were appended). This is a compatibility fixture, not a second
 * maintained public header. Never regenerate or update it for ABI v1 changes.
 */
#ifndef CALIBER_V1_PREFIX_FIXTURE_H
#define CALIBER_V1_PREFIX_FIXTURE_H

#include <stddef.h>
#include <stdint.h>

typedef enum CaliberStatus {
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
    CALIBER_STATUS_INTERNAL = 10
} CaliberStatus;

typedef struct CaliberContext CaliberContext;

typedef struct CaliberContextConfig {
    uint32_t struct_size;
    size_t max_command_bytes;
    size_t max_publication_bytes;
    size_t max_resource_bytes;
    size_t max_resources;
    size_t telemetry_width;
    size_t max_pending_commands;
} CaliberContextConfig;

typedef struct CaliberStatePublication {
    uint64_t revision;
    uint32_t schema;
    uint32_t reserved;
    const uint8_t *data;
    size_t len;
    void *lease;
} CaliberStatePublication;

typedef struct CaliberResourceView {
    uint64_t resource_id;
    uint64_t generation;
    const uint8_t *data;
    size_t len;
    void *lease;
} CaliberResourceView;

typedef struct CaliberTelemetryInfo {
    uint64_t sequence;
    uint32_t schema;
    uint32_t reserved;
    size_t value_count;
    size_t value_size;
} CaliberTelemetryInfo;

/* Exact v1 table prefix that clients compiled before wake-wait extensions use. */
typedef struct CaliberApiV1 {
    uint32_t abi_version;
    uint32_t struct_size;
    CaliberStatus (*context_create)(const CaliberContextConfig *, CaliberContext **);
    void (*context_destroy)(CaliberContext *);
    CaliberStatus (*context_dispatch)(const CaliberContext *, const uint8_t *, size_t);
    CaliberStatus (*context_peek_command)(const CaliberContext *, size_t *);
    CaliberStatus (*context_take_command)(const CaliberContext *, uint8_t *, size_t, size_t *);
    CaliberStatus (*context_publish_state)(const CaliberContext *, uint32_t, const uint8_t *, size_t, uint64_t *);
    CaliberStatus (*context_read_latest_state)(const CaliberContext *, CaliberStatePublication *);
    void (*state_publication_release)(CaliberStatePublication *);
    CaliberStatus (*context_map_resource)(const CaliberContext *, uint64_t, uint64_t, CaliberResourceView *);
    void (*resource_release)(CaliberResourceView *);
    CaliberStatus (*context_publish_resource)(const CaliberContext *, const uint8_t *, size_t, uint64_t *, uint64_t *);
    CaliberStatus (*context_release_resource)(const CaliberContext *, uint64_t, uint64_t);
    CaliberStatus (*context_publish_telemetry)(const CaliberContext *, const size_t *, size_t);
    CaliberStatus (*context_read_latest_telemetry)(const CaliberContext *, size_t *, size_t, CaliberTelemetryInfo *);
    CaliberStatus (*context_wake_sequence)(const CaliberContext *, uint64_t *);
} CaliberApiV1;

#endif
