#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "caliber.h"

/* Return the failing source line, while still destroying a live context. */
#define CHECK(condition) do { if (!(condition)) { result = __LINE__; goto done; } } while (0)

int caliber_command_capacity_client_run(const CaliberApiV1 *api) {
    CaliberContext *context = NULL;
    CaliberContextConfig config = {0};
    const uint8_t first[] = {0x12, 0x34, 0x56};
    const uint8_t second[] = {0x78};
    uint8_t out[sizeof(first)] = {0xaa, 0xaa, 0xaa};
    size_t len = 99;
    uint64_t before = 0, accepted = 0, after = 0;
    int result = 0;

    config.struct_size = (uint32_t)sizeof(config);
    config.max_pending_commands = 1;
    CHECK(api->context_create(&config, &context) == CALIBER_STATUS_OK);
    CHECK(api->context_wake_sequence(context, &before) == CALIBER_STATUS_OK);
    CHECK(api->context_dispatch(context, first, sizeof(first)) == CALIBER_STATUS_OK);
    CHECK(api->context_wake_sequence(context, &accepted) == CALIBER_STATUS_OK);
    CHECK(accepted != before);

    CHECK(api->context_peek_command(context, &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(first));
    CHECK(api->context_dispatch(context, second, sizeof(second)) == CALIBER_STATUS_QUEUE_FULL);
    CHECK(api->context_take_command(context, out, sizeof(out) - 1, &len) == CALIBER_STATUS_BUFFER_TOO_SMALL);
    CHECK(len == sizeof(first));
    CHECK(out[0] == 0xaa && out[1] == 0xaa && out[2] == 0xaa);
    CHECK(api->context_dispatch(context, second, sizeof(second)) == CALIBER_STATUS_QUEUE_FULL);
    CHECK(api->context_wake_sequence(context, &after) == CALIBER_STATUS_OK);
    CHECK(after == accepted);
    CHECK(api->context_take_command(context, out, sizeof(out), &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(first) && memcmp(out, first, len) == 0);
    CHECK(api->context_wake_sequence(context, &after) == CALIBER_STATUS_OK);
    CHECK(after == accepted);

    CHECK(api->context_dispatch(context, second, sizeof(second)) == CALIBER_STATUS_OK);
    CHECK(api->context_wake_sequence(context, &after) == CALIBER_STATUS_OK);
    CHECK(after != accepted);
    CHECK(api->context_take_command(context, NULL, 0, &len) == CALIBER_STATUS_BUFFER_TOO_SMALL);
    CHECK(len == sizeof(second));
    CHECK(api->context_dispatch(context, first, sizeof(first)) == CALIBER_STATUS_QUEUE_FULL);
    CHECK(api->context_take_command(context, out, sizeof(out), &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(second) && memcmp(out, second, len) == 0);
    CHECK(api->context_peek_command(context, &len) == CALIBER_STATUS_UNAVAILABLE);

done:
    if (context != NULL) api->context_destroy(context);
    return result;
}

int caliber_command_fifo_client_run(const CaliberApiV1 *api) {
    CaliberContext *context = NULL;
    CaliberContextConfig config = {0};
    const uint8_t first[] = {1, 2, 3};
    const uint8_t last[] = {4, 5};
    const uint8_t appended[] = {6};
    uint8_t out[sizeof(first)] = {0};
    size_t len = 99;
    int result = 0;

    config.struct_size = (uint32_t)sizeof(config);
    config.max_pending_commands = 3;
    CHECK(api->context_create(&config, &context) == CALIBER_STATUS_OK);
    CHECK(api->context_dispatch(NULL, NULL, 0) == CALIBER_STATUS_INVALID_HANDLE);
    CHECK(api->context_peek_command(NULL, &len) == CALIBER_STATUS_INVALID_HANDLE);
    CHECK(api->context_take_command(NULL, NULL, 0, &len) == CALIBER_STATUS_INVALID_HANDLE);
    CHECK(api->context_dispatch(context, NULL, 1) == CALIBER_STATUS_INVALID_ARGUMENT);
    CHECK(api->context_peek_command(context, NULL) == CALIBER_STATUS_INVALID_ARGUMENT);
    CHECK(api->context_take_command(context, out, sizeof(out), NULL) == CALIBER_STATUS_INVALID_ARGUMENT);
    CHECK(api->context_take_command(context, NULL, 1, &len) == CALIBER_STATUS_INVALID_ARGUMENT);
    CHECK(api->context_take_command(context, NULL, 0, &len) == CALIBER_STATUS_UNAVAILABLE);
    CHECK(len == 99);

    CHECK(api->context_dispatch(context, first, sizeof(first)) == CALIBER_STATUS_OK);
    CHECK(api->context_dispatch(context, NULL, 0) == CALIBER_STATUS_OK);
    CHECK(api->context_dispatch(context, last, sizeof(last)) == CALIBER_STATUS_OK);
    CHECK(api->context_take_command(context, NULL, 0, &len) == CALIBER_STATUS_BUFFER_TOO_SMALL);
    CHECK(len == sizeof(first));
    CHECK(api->context_peek_command(context, &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(first));
    CHECK(api->context_take_command(context, out, sizeof(out), &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(first) && memcmp(out, first, len) == 0);
    CHECK(api->context_dispatch(context, appended, sizeof(appended)) == CALIBER_STATUS_OK);
    CHECK(api->context_peek_command(context, &len) == CALIBER_STATUS_OK);
    CHECK(len == 0);
    CHECK(api->context_take_command(context, NULL, 0, &len) == CALIBER_STATUS_OK);
    CHECK(len == 0);
    CHECK(api->context_take_command(context, out, sizeof(out), &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(last) && memcmp(out, last, len) == 0);
    CHECK(api->context_take_command(context, out, sizeof(out), &len) == CALIBER_STATUS_OK);
    CHECK(len == sizeof(appended) && memcmp(out, appended, len) == 0);
    len = 99;
    CHECK(api->context_peek_command(context, &len) == CALIBER_STATUS_UNAVAILABLE);
    CHECK(len == 99);

done:
    if (context != NULL) api->context_destroy(context);
    return result;
}
