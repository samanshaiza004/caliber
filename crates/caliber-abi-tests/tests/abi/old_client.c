#include <stddef.h>
#include <stdint.h>
#include "caliber_v1_prefix.h"

#if defined(_WIN32)
#include <windows.h>
typedef HMODULE CaliberModule;
static CaliberModule open_module(const char *path) { return LoadLibraryA(path); }
static void *find_symbol(CaliberModule module, const char *name) { return (void *)GetProcAddress(module, name); }
static void close_module(CaliberModule module) { if (module != NULL) FreeLibrary(module); }
#else
#include <dlfcn.h>
typedef void *CaliberModule;
static CaliberModule open_module(const char *path) { return dlopen(path, RTLD_NOW | RTLD_LOCAL); }
static void *find_symbol(CaliberModule module, const char *name) { return dlsym(module, name); }
static void close_module(CaliberModule module) { if (module != NULL) dlclose(module); }
#endif

typedef const CaliberApiV1 *(*GetApiProc)(uint32_t);

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(CaliberContextConfig) == 56, "frozen config size changed");
_Static_assert(offsetof(CaliberContextConfig, max_command_bytes) == 8, "frozen config prefix moved");
_Static_assert(sizeof(CaliberStatePublication) == 40, "frozen state record size changed");
_Static_assert(offsetof(CaliberStatePublication, data) == 16, "frozen state data offset changed");
_Static_assert(sizeof(CaliberResourceView) == 40, "frozen resource record size changed");
_Static_assert(offsetof(CaliberResourceView, data) == 16, "frozen resource data offset changed");
_Static_assert(sizeof(CaliberTelemetryInfo) == 32, "frozen telemetry record size changed");
_Static_assert(offsetof(CaliberTelemetryInfo, value_count) == 16, "frozen telemetry prefix moved");
_Static_assert(sizeof(CaliberApiV1) == 128, "frozen ABI v1 table size changed");
_Static_assert(offsetof(CaliberApiV1, context_wake_sequence) == 120, "frozen ABI v1 prefix moved");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(CaliberContextConfig) == 28, "frozen config size changed");
_Static_assert(offsetof(CaliberContextConfig, max_command_bytes) == 4, "frozen config prefix moved");
_Static_assert(sizeof(CaliberStatePublication) == 28, "frozen state record size changed");
_Static_assert(offsetof(CaliberStatePublication, data) == 16, "frozen state data offset changed");
_Static_assert(sizeof(CaliberResourceView) == 28, "frozen resource record size changed");
_Static_assert(offsetof(CaliberResourceView, data) == 16, "frozen resource data offset changed");
_Static_assert(sizeof(CaliberTelemetryInfo) == 24, "frozen telemetry record size changed");
_Static_assert(offsetof(CaliberTelemetryInfo, value_count) == 16, "frozen telemetry prefix moved");
_Static_assert(sizeof(CaliberApiV1) == 68, "frozen ABI v1 table size changed");
_Static_assert(offsetof(CaliberApiV1, context_wake_sequence) == 64, "frozen ABI v1 prefix moved");
#else
#error Unsupported pointer width for the frozen Caliber ABI fixture
#endif

int caliber_old_v1_client_run(const char *library_path) {
    CaliberModule module = open_module(library_path);
    CaliberContext *context = NULL;
    int result = 0;
    if (module == NULL) return 1;
    GetApiProc get_api = (GetApiProc)find_symbol(module, "caliber_get_api");
    if (get_api == NULL) { result = 2; goto done; }
    const CaliberApiV1 *api = get_api(1);
    if (api == NULL || api->abi_version != 1 || api->struct_size < sizeof(CaliberApiV1)) { result = 3; goto done; }
    if (api->context_create == NULL || api->context_destroy == NULL || api->context_dispatch == NULL ||
        api->context_peek_command == NULL || api->context_take_command == NULL || api->context_publish_state == NULL ||
        api->context_read_latest_state == NULL || api->state_publication_release == NULL || api->context_map_resource == NULL ||
        api->resource_release == NULL || api->context_publish_resource == NULL || api->context_release_resource == NULL ||
        api->context_publish_telemetry == NULL || api->context_read_latest_telemetry == NULL || api->context_wake_sequence == NULL) {
        result = 4; goto done;
    }
    if (CALIBER_STATUS_OK != 0 || CALIBER_STATUS_INVALID_ARGUMENT != 1 || CALIBER_STATUS_INVALID_HANDLE != 2 ||
        CALIBER_STATUS_BUFFER_TOO_SMALL != 3 || CALIBER_STATUS_LIMIT_EXCEEDED != 4 || CALIBER_STATUS_NOT_FOUND != 5 ||
        CALIBER_STATUS_STALE != 6 || CALIBER_STATUS_UNAVAILABLE != 7 || CALIBER_STATUS_QUEUE_FULL != 8 ||
        CALIBER_STATUS_UNSUPPORTED_VERSION != 9 || CALIBER_STATUS_INTERNAL != 10) { result = 5; goto done; }

    CaliberContextConfig config = {0};
    config.struct_size = (uint32_t)sizeof(config);
    config.max_command_bytes = 3;
    config.max_publication_bytes = 5;
    config.max_resource_bytes = 6;
    config.max_resources = 1;
    config.telemetry_width = 2;
    config.max_pending_commands = 1;
    if (api->context_create(&config, &context) != CALIBER_STATUS_OK || context == NULL) { result = 6; goto done; }

    const uint8_t command[] = {'o', 'l', 'd'};
    if (api->context_dispatch(context, command, sizeof(command)) != CALIBER_STATUS_OK ||
        api->context_dispatch(context, command, sizeof(command)) != CALIBER_STATUS_QUEUE_FULL) { result = 7; goto done; }
    size_t command_len = 0;
    uint8_t command_copy[8] = {0};
    if (api->context_peek_command(context, &command_len) != CALIBER_STATUS_OK || command_len != sizeof(command)) { result = 8; goto done; }
    if (api->context_take_command(context, command_copy, sizeof(command_copy), &command_len) != CALIBER_STATUS_OK ||
        command_len != sizeof(command) || command_copy[0] != 'o' || command_copy[2] != 'd') { result = 9; goto done; }
    const uint8_t oversized_command[] = {'o', 'l', 'd', '!'};
    if (api->context_dispatch(context, oversized_command, sizeof(oversized_command)) != CALIBER_STATUS_LIMIT_EXCEEDED) { result = 18; goto done; }

    uint64_t revision = 0;
    const uint8_t state[] = {'s', 't', 'a', 't', 'e'};
    if (api->context_publish_state(context, 7, state, sizeof(state), &revision) != CALIBER_STATUS_OK) { result = 10; goto done; }
    CaliberStatePublication publication = {0};
    if (api->context_read_latest_state(context, &publication) != CALIBER_STATUS_OK || publication.revision != revision ||
        publication.schema != 7 || publication.len != sizeof(state) || publication.data[0] != 's') { result = 11; goto done; }
    api->state_publication_release(&publication);
    const uint8_t oversized_state[] = {'s', 't', 'a', 't', 'e', '!'};
    if (api->context_publish_state(context, 7, oversized_state, sizeof(oversized_state), &revision) != CALIBER_STATUS_LIMIT_EXCEEDED) {
        result = 19; goto done;
    }

    uint64_t resource_id = 0, generation = 0;
    if (api->context_publish_resource(context, state, sizeof(state), &resource_id, &generation) != CALIBER_STATUS_OK) { result = 12; goto done; }
    CaliberResourceView view = {0};
    if (api->context_map_resource(context, resource_id, generation, &view) != CALIBER_STATUS_OK || view.len != sizeof(state) || view.data[4] != 'e') { result = 13; goto done; }
    api->resource_release(&view);
    if (api->context_release_resource(context, resource_id, generation) != CALIBER_STATUS_OK) { result = 14; goto done; }
    if (api->context_publish_resource(context, state, sizeof(state), &resource_id, &generation) != CALIBER_STATUS_QUEUE_FULL) { result = 20; goto done; }

    const size_t telemetry[] = {17, 23};
    if (api->context_publish_telemetry(context, telemetry, 2) != CALIBER_STATUS_OK) { result = 15; goto done; }
    size_t telemetry_copy[2] = {0};
    CaliberTelemetryInfo telemetry_info = {0};
    if (api->context_read_latest_telemetry(context, telemetry_copy, 2, &telemetry_info) != CALIBER_STATUS_OK ||
        telemetry_copy[0] != 17 || telemetry_copy[1] != 23 || telemetry_info.value_count != 2 || telemetry_info.value_size != sizeof(size_t)) {
        result = 16; goto done;
    }
    uint64_t wake_sequence = 0;
    if (api->context_wake_sequence(context, &wake_sequence) != CALIBER_STATUS_OK || wake_sequence == 0) { result = 17; goto done; }

done:
    if (context != NULL) api->context_destroy(context);
    close_module(module);
    return result;
}
