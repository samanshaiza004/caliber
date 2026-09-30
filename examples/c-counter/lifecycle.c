/*
 * Canonical Caliber ABI v1 C lifecycle example.
 *
 * Build against ../../include/caliber.h and pass the built shared library path.
 * The application-level command interpretation and state schema below are
 * deliberately illustrative; Caliber transports bytes and owns snapshots,
 * leases, resource handles, and wake coordination.
 */
#include "caliber.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
typedef HMODULE CaliberModule;
typedef HANDLE ExampleThread;
static CaliberModule open_module(const char *path) { return LoadLibraryA(path); }
static void *find_symbol(CaliberModule module, const char *name) {
    return (void *)GetProcAddress(module, name);
}
static void close_module(CaliberModule module) {
    if (module != NULL) FreeLibrary(module);
}
#else
#include <dlfcn.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
typedef void *CaliberModule;
typedef pthread_t ExampleThread;
static CaliberModule open_module(const char *path) {
    return dlopen(path, RTLD_NOW | RTLD_LOCAL);
}
static void *find_symbol(CaliberModule module, const char *name) {
    return dlsym(module, name);
}
static void close_module(CaliberModule module) {
    if (module != NULL) dlclose(module);
}
#endif

typedef const CaliberApiV1 *(*GetApiProc)(uint32_t);

typedef struct Waiter {
    const CaliberApiV1 *api;
    const CaliberContext *context;
    uint64_t observed_sequence;
    uint64_t result_sequence;
    CaliberStatus status;
#if defined(_WIN32)
    volatile LONG started;
#else
    atomic_int started;
#endif
} Waiter;

static void mark_waiter_started(Waiter *waiter) {
#if defined(_WIN32)
    InterlockedExchange(&waiter->started, 1);
#else
    atomic_store_explicit(&waiter->started, 1, memory_order_release);
#endif
}

static int waiter_has_started(const Waiter *waiter) {
#if defined(_WIN32)
    return InterlockedCompareExchange((volatile LONG *)&waiter->started, 0, 0) != 0;
#else
    return atomic_load_explicit(&waiter->started, memory_order_acquire) != 0;
#endif
}

static void yield_thread(void) {
#if defined(_WIN32)
    SwitchToThread();
#else
    sched_yield();
#endif
}

static void wait_for_change(Waiter *waiter) {
    mark_waiter_started(waiter);
    waiter->status = waiter->api->context_wait_wake(
        waiter->context, waiter->observed_sequence, &waiter->result_sequence);
}

#if defined(_WIN32)
static DWORD WINAPI waiter_entry(LPVOID argument) {
    wait_for_change((Waiter *)argument);
    return 0;
}
static int start_waiter(Waiter *waiter, ExampleThread *thread) {
    *thread = CreateThread(NULL, 0, waiter_entry, waiter, 0, NULL);
    return *thread != NULL;
}
static int join_waiter(ExampleThread thread) {
    DWORD result = WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
    return result == WAIT_OBJECT_0;
}
#else
static void *waiter_entry(void *argument) {
    wait_for_change((Waiter *)argument);
    return NULL;
}
static int start_waiter(Waiter *waiter, ExampleThread *thread) {
    return pthread_create(thread, NULL, waiter_entry, waiter) == 0;
}
static int join_waiter(ExampleThread thread) {
    return pthread_join(thread, NULL) == 0;
}
#endif

#define FIELD_PRESENT(api, field) \
    ((api)->struct_size >= offsetof(CaliberApiV1, field) + sizeof((api)->field))

#define REQUIRE_STATUS(expression, expected, operation)                         \
    do {                                                                         \
        CaliberStatus actual_status = (expression);                              \
        if (actual_status != (expected)) {                                       \
            fprintf(stderr, "%s returned Caliber status %d (expected %d)\n",   \
                    (operation), (int)actual_status, (int)(expected));            \
            goto cleanup;                                                        \
        }                                                                        \
    } while (0)

int main(int argc, char **argv) {
    int result = 1;
    CaliberModule module = NULL;
    CaliberContext *context = NULL;
    const CaliberApiV1 *api = NULL;
    ExampleThread thread;
    int thread_joinable = 0;
    Waiter waiter;
    CaliberStatePublication state_view = {0};
    CaliberResourceView resource_view = {0};
    uint64_t resource_id = 0;
    uint64_t resource_generation = 0;
    int resource_registered = 0;

    if (argc != 2) {
        fprintf(stderr, "usage: %s PATH_TO_CALIBER_SHARED_LIBRARY\n", argv[0]);
        return 2;
    }
    module = open_module(argv[1]);
    if (module == NULL) {
        fprintf(stderr, "could not load Caliber shared library: %s\n", argv[1]);
        goto cleanup;
    }
    GetApiProc get_api = (GetApiProc)find_symbol(module, "caliber_get_api");
    if (get_api == NULL) {
        fprintf(stderr, "shared library does not export caliber_get_api\n");
        goto cleanup;
    }
    api = get_api(CALIBER_ABI_VERSION_1);
    if (api == NULL || api->abi_version != CALIBER_ABI_VERSION_1) {
        fprintf(stderr, "shared library does not provide Caliber ABI v1\n");
        goto cleanup;
    }

    /* Check the table prefix before reading any function pointer we use. */
#define REQUIRE_FIELD(field)                                                    \
    do {                                                                         \
        if (!FIELD_PRESENT(api, field) || api->field == NULL) {                  \
            fprintf(stderr, "Caliber ABI v1 table is missing %s\n", #field);    \
            goto cleanup;                                                        \
        }                                                                        \
    } while (0)
    REQUIRE_FIELD(context_create);
    REQUIRE_FIELD(context_destroy);
    REQUIRE_FIELD(context_dispatch);
    REQUIRE_FIELD(context_peek_command);
    REQUIRE_FIELD(context_take_command);
    REQUIRE_FIELD(context_publish_state);
    REQUIRE_FIELD(context_read_latest_state);
    REQUIRE_FIELD(state_publication_release);
    REQUIRE_FIELD(context_map_resource);
    REQUIRE_FIELD(resource_release);
    REQUIRE_FIELD(context_publish_resource);
    REQUIRE_FIELD(context_release_resource);
    REQUIRE_FIELD(context_wake_sequence);
    REQUIRE_FIELD(context_wait_wake);
    REQUIRE_FIELD(context_stop_wake_waiters);
#undef REQUIRE_FIELD

    CaliberContextConfig config = {0};
    config.struct_size = (uint32_t)sizeof(config);
    REQUIRE_STATUS(api->context_create(&config, &context), CALIBER_STATUS_OK,
                   "context_create");

    /* The application dispatches a semantic command; Caliber copies its bytes. */
    static const uint8_t command[] = "{\"op\":\"open\",\"path\":\"notes.md\"}";
    REQUIRE_STATUS(api->context_dispatch(context, command, sizeof(command) - 1),
                   CALIBER_STATUS_OK, "context_dispatch");
    size_t command_size = 0;
    REQUIRE_STATUS(api->context_peek_command(context, &command_size),
                   CALIBER_STATUS_OK, "context_peek_command");
    uint8_t command_copy[128];
    size_t command_taken = 0;
    if (command_size > sizeof(command_copy)) {
        fprintf(stderr, "example command is larger than its local buffer\n");
        goto cleanup;
    }
    REQUIRE_STATUS(api->context_take_command(context, command_copy,
                                             sizeof(command_copy), &command_taken),
                   CALIBER_STATUS_OK, "context_take_command");
    if (command_taken != sizeof(command) - 1 ||
        memcmp(command_copy, command, command_taken) != 0) {
        fprintf(stderr, "application received different command bytes\n");
        goto cleanup;
    }

    /* A tiny immutable resource demonstrates both the mapped lease and retire. */
    static const uint8_t resource[] = "bounded source window";
    REQUIRE_STATUS(api->context_publish_resource(context, resource,
                                                 sizeof(resource) - 1,
                                                 &resource_id,
                                                 &resource_generation),
                   CALIBER_STATUS_OK, "context_publish_resource");
    resource_registered = 1;
    REQUIRE_STATUS(api->context_map_resource(context, resource_id,
                                             resource_generation, &resource_view),
                   CALIBER_STATUS_OK, "context_map_resource");
    if (resource_view.len != sizeof(resource) - 1 ||
        memcmp(resource_view.data, resource, resource_view.len) != 0) {
        fprintf(stderr, "mapped resource does not match published bytes\n");
        goto cleanup;
    }
    api->resource_release(&resource_view);
    REQUIRE_STATUS(api->context_release_resource(context, resource_id,
                                                 resource_generation),
                   CALIBER_STATUS_OK, "context_release_resource");
    resource_registered = 0;

    /* The application processes the command and prepares its own state. */
    static const uint8_t state[] = "{\"document\":\"notes.md\",\"open\":true}";
    uint64_t observed_sequence = 0;
    REQUIRE_STATUS(api->context_wake_sequence(context, &observed_sequence),
                   CALIBER_STATUS_OK, "context_wake_sequence");
    waiter.api = api;
    waiter.context = context;
    waiter.observed_sequence = observed_sequence;
    waiter.result_sequence = 0;
    waiter.status = CALIBER_STATUS_INTERNAL;
#if defined(_WIN32)
    waiter.started = 0;
#else
    atomic_init(&waiter.started, 0);
#endif
    if (!start_waiter(&waiter, &thread)) {
        fprintf(stderr, "could not start publication waiter thread\n");
        goto cleanup;
    }
    thread_joinable = 1;
    while (!waiter_has_started(&waiter)) yield_thread();

    uint64_t state_revision = 0;
    REQUIRE_STATUS(api->context_publish_state(context, 1, state,
                                              sizeof(state) - 1, &state_revision),
                   CALIBER_STATUS_OK, "context_publish_state");
    if (!join_waiter(thread)) {
        fprintf(stderr, "could not join publication waiter thread\n");
        thread_joinable = 0;
        goto cleanup;
    }
    thread_joinable = 0;
    if (waiter.status != CALIBER_STATUS_OK ||
        waiter.result_sequence == observed_sequence) {
        fprintf(stderr, "waiter did not observe the published state change\n");
        goto cleanup;
    }

    REQUIRE_STATUS(api->context_read_latest_state(context, &state_view),
                   CALIBER_STATUS_OK, "context_read_latest_state");
    if (state_view.revision != state_revision || state_view.schema != 1 ||
        state_view.len != sizeof(state) - 1 ||
        memcmp(state_view.data, state, state_view.len) != 0) {
        fprintf(stderr, "state lease does not match the latest publication\n");
        goto cleanup;
    }
    api->state_publication_release(&state_view);

    /* Stop and join a waiter before destroying its context. */
    waiter.api = api;
    waiter.context = context;
    waiter.observed_sequence = waiter.result_sequence;
    waiter.result_sequence = 0;
    waiter.status = CALIBER_STATUS_INTERNAL;
#if defined(_WIN32)
    waiter.started = 0;
#else
    atomic_init(&waiter.started, 0);
#endif
    if (!start_waiter(&waiter, &thread)) {
        fprintf(stderr, "could not start shutdown waiter thread\n");
        goto cleanup;
    }
    thread_joinable = 1;
    while (!waiter_has_started(&waiter)) yield_thread();
    REQUIRE_STATUS(api->context_stop_wake_waiters(context), CALIBER_STATUS_OK,
                   "context_stop_wake_waiters");
    if (!join_waiter(thread)) {
        fprintf(stderr, "could not join shutdown waiter thread\n");
        thread_joinable = 0;
        goto cleanup;
    }
    thread_joinable = 0;
    if (waiter.status != CALIBER_STATUS_STOPPED) {
        fprintf(stderr, "shutdown waiter returned status %d, expected STOPPED\n",
                (int)waiter.status);
        goto cleanup;
    }

    printf("Caliber C lifecycle example passed (state revision %llu).\n",
           (unsigned long long)state_revision);
    result = 0;

cleanup:
    if (api != NULL && context != NULL && api->context_stop_wake_waiters != NULL)
        (void)api->context_stop_wake_waiters(context);
    if (thread_joinable) {
        (void)join_waiter(thread);
    }
    if (api != NULL && resource_view.lease != NULL)
        api->resource_release(&resource_view);
    if (api != NULL && state_view.lease != NULL)
        api->state_publication_release(&state_view);
    if (api != NULL && context != NULL && resource_registered)
        (void)api->context_release_resource(context, resource_id,
                                            resource_generation);
    if (api != NULL && context != NULL && api->context_destroy != NULL)
        api->context_destroy(context);
    close_module(module);
    return result;
}
