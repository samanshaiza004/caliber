#include <stddef.h>
#include <stdint.h>
#include "caliber.h"

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

_Static_assert(sizeof(CaliberStatus) == sizeof(int32_t), "CaliberStatus must be exactly int32_t");

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(CaliberApiV1) == 144, "current ABI v1 table size changed unexpectedly");
_Static_assert(offsetof(CaliberApiV1, context_wake_sequence) == 120, "existing ABI v1 prefix moved");
_Static_assert(offsetof(CaliberApiV1, context_wait_wake) == 128, "wait entry must append to v1");
_Static_assert(offsetof(CaliberApiV1, context_stop_wake_waiters) == 136, "stop entry must append to v1");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(CaliberApiV1) == 76, "current ABI v1 table size changed unexpectedly");
_Static_assert(offsetof(CaliberApiV1, context_wake_sequence) == 64, "existing ABI v1 prefix moved");
_Static_assert(offsetof(CaliberApiV1, context_wait_wake) == 68, "wait entry must append to v1");
_Static_assert(offsetof(CaliberApiV1, context_stop_wake_waiters) == 72, "stop entry must append to v1");
#else
#error Unsupported pointer width for Caliber ABI v1
#endif

int caliber_current_v1_client_run(const char *library_path) {
    CaliberModule module = open_module(library_path);
    CaliberContext *context = NULL;
    int result = 0;
    if (module == NULL) return 1;
    GetApiProc get_api = (GetApiProc)find_symbol(module, "caliber_get_api");
    if (get_api == NULL) { result = 2; goto done; }
    const CaliberApiV1 *api = get_api(1);
    if (api == NULL || api->abi_version != 1 || api->struct_size < sizeof(CaliberApiV1)) { result = 3; goto done; }
    if (api->context_wait_wake == NULL || api->context_stop_wake_waiters == NULL) { result = 4; goto done; }
    if (CALIBER_STATUS_OK != 0 || CALIBER_STATUS_INVALID_ARGUMENT != 1 || CALIBER_STATUS_INVALID_HANDLE != 2 ||
        CALIBER_STATUS_BUFFER_TOO_SMALL != 3 || CALIBER_STATUS_LIMIT_EXCEEDED != 4 || CALIBER_STATUS_NOT_FOUND != 5 ||
        CALIBER_STATUS_STALE != 6 || CALIBER_STATUS_UNAVAILABLE != 7 || CALIBER_STATUS_QUEUE_FULL != 8 ||
        CALIBER_STATUS_UNSUPPORTED_VERSION != 9 || CALIBER_STATUS_INTERNAL != 10 || CALIBER_STATUS_STOPPED != 11) { result = 5; goto done; }

    CaliberContextConfig config = {0};
    config.struct_size = (uint32_t)sizeof(config);
    config.telemetry_width = 1;
    CaliberStatus status = api->context_create(&config, &context);
    if (status != CALIBER_STATUS_OK || context == NULL) { result = 6; goto done; }
    const size_t telemetry_in[] = {(size_t)0x12345};
    size_t telemetry_out[] = {0};
    CaliberTelemetryInfo telemetry_info = {0};
    if (api->context_publish_telemetry(context, telemetry_in, 1) != CALIBER_STATUS_OK) { result = 12; goto done; }
    if (api->context_read_latest_telemetry(context, telemetry_out, 1, &telemetry_info) != CALIBER_STATUS_OK ||
        telemetry_out[0] != telemetry_in[0] || telemetry_info.sequence != 1 || telemetry_info.schema != 0 ||
        telemetry_info.reserved != 0 || telemetry_info.value_count != 1 ||
        telemetry_info.value_size != sizeof(size_t)) { result = 13; goto done; }
    uint64_t before = 0, after = 0;
    if (api->context_wake_sequence(context, &before) != CALIBER_STATUS_OK) { result = 7; goto done; }
    const uint8_t command[] = {'n', 'e', 'w'};
    if (api->context_dispatch(context, command, sizeof(command)) != CALIBER_STATUS_OK) { result = 8; goto done; }
    if (api->context_wait_wake(context, before, &after) != CALIBER_STATUS_OK || after == before) { result = 9; goto done; }
    if (api->context_stop_wake_waiters(context) != CALIBER_STATUS_OK) { result = 10; goto done; }
    uint64_t stopped_sequence = 0;
    if (api->context_wait_wake(context, after, &stopped_sequence) != CALIBER_STATUS_STOPPED) { result = 11; goto done; }

done:
    if (context != NULL) api->context_destroy(context);
    close_module(module);
    return result;
}
