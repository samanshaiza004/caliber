#include <stddef.h>
#include "caliber.h"

/* Keep in sync with the ordered Rust expectation in tests/abi_compat.rs. */
void caliber_abi_c_layout(size_t out[48]) {
    const size_t values[48] = {
        sizeof(CaliberStatus),
        sizeof(CaliberContextConfig),
        offsetof(CaliberContextConfig, struct_size),
        offsetof(CaliberContextConfig, max_command_bytes),
        offsetof(CaliberContextConfig, max_publication_bytes),
        offsetof(CaliberContextConfig, max_resource_bytes),
        offsetof(CaliberContextConfig, max_resources),
        offsetof(CaliberContextConfig, telemetry_width),
        offsetof(CaliberContextConfig, max_pending_commands),
        sizeof(CaliberStatePublication),
        offsetof(CaliberStatePublication, revision),
        offsetof(CaliberStatePublication, schema),
        offsetof(CaliberStatePublication, reserved),
        offsetof(CaliberStatePublication, data),
        offsetof(CaliberStatePublication, len),
        offsetof(CaliberStatePublication, lease),
        sizeof(CaliberResourceView),
        offsetof(CaliberResourceView, resource_id),
        offsetof(CaliberResourceView, generation),
        offsetof(CaliberResourceView, data),
        offsetof(CaliberResourceView, len),
        offsetof(CaliberResourceView, lease),
        sizeof(CaliberTelemetryInfo),
        offsetof(CaliberTelemetryInfo, sequence),
        offsetof(CaliberTelemetryInfo, schema),
        offsetof(CaliberTelemetryInfo, reserved),
        offsetof(CaliberTelemetryInfo, value_count),
        offsetof(CaliberTelemetryInfo, value_size),
        sizeof(CaliberApiV1),
        offsetof(CaliberApiV1, abi_version),
        offsetof(CaliberApiV1, struct_size),
        offsetof(CaliberApiV1, context_create),
        offsetof(CaliberApiV1, context_destroy),
        offsetof(CaliberApiV1, context_dispatch),
        offsetof(CaliberApiV1, context_peek_command),
        offsetof(CaliberApiV1, context_take_command),
        offsetof(CaliberApiV1, context_publish_state),
        offsetof(CaliberApiV1, context_read_latest_state),
        offsetof(CaliberApiV1, state_publication_release),
        offsetof(CaliberApiV1, context_map_resource),
        offsetof(CaliberApiV1, resource_release),
        offsetof(CaliberApiV1, context_publish_resource),
        offsetof(CaliberApiV1, context_release_resource),
        offsetof(CaliberApiV1, context_publish_telemetry),
        offsetof(CaliberApiV1, context_read_latest_telemetry),
        offsetof(CaliberApiV1, context_wake_sequence),
        offsetof(CaliberApiV1, context_wait_wake),
        offsetof(CaliberApiV1, context_stop_wake_waiters)
    };
    size_t i;
    for (i = 0; i < 48; ++i) out[i] = values[i];
}
