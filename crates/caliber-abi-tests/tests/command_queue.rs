use caliber_ffi::{
    CALIBER_ABI_VERSION_1, CaliberApiV1, CaliberContext, CaliberContextConfig, CaliberStatus,
    caliber_get_api,
};
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

#[link(name = "caliber_abi_clients", kind = "static")]
unsafe extern "C" {
    fn caliber_command_capacity_client_run(api: *const CaliberApiV1) -> c_int;
    fn caliber_command_fifo_client_run(api: *const CaliberApiV1) -> c_int;
}

#[test]
fn c_client_fifo_and_null_pointer_contracts() {
    let api = caliber_get_api(CALIBER_ABI_VERSION_1);
    assert!(!api.is_null());
    // SAFETY: the API is Caliber's static v1 table; the C client owns its context.
    let result = unsafe { caliber_command_fifo_client_run(api) };
    assert_eq!(
        result, 0,
        "C queue client failed at command_queue.c:{result}"
    );
}

#[test]
fn concurrent_c_abi_producers_respect_capacity_one_after_peek_and_short_take() {
    const PRODUCERS: usize = 8;

    // SAFETY: version 1 has a static function table for the life of the process.
    let api = unsafe { &*caliber_get_api(CALIBER_ABI_VERSION_1) };
    let dispatch = api.context_dispatch.unwrap();
    let config = CaliberContextConfig {
        max_pending_commands: 1,
        ..CaliberContextConfig::default()
    };
    let mut context = ptr::null_mut();
    // SAFETY: config and the output pointer are valid for the call.
    assert_eq!(
        unsafe { api.context_create.unwrap()(&config, &mut context) },
        CaliberStatus::Ok
    );
    assert!(!context.is_null());
    let address = context as usize;
    let dispatch_together = || {
        let start = Barrier::new(PRODUCERS);
        thread::scope(|scope| {
            let threads: Vec<_> = (0..PRODUCERS)
                .map(|id| {
                    let start = &start;
                    scope.spawn(move || {
                        let command = [id as u8, 0x5a];
                        start.wait();
                        // SAFETY: context remains live until all threads join, and command is local.
                        unsafe {
                            dispatch(
                                address as *const CaliberContext,
                                command.as_ptr(),
                                command.len(),
                            )
                        }
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        })
    };

    let first = dispatch_together();
    let mut len = usize::MAX;
    // SAFETY: the context and output pointers remain valid through each call.
    let peek = unsafe { api.context_peek_command.unwrap()(context, &mut len) };
    let peek_len = len;
    let after_peek = dispatch_together();
    let mut out = [0u8; 2];
    let short =
        unsafe { api.context_take_command.unwrap()(context, out.as_mut_ptr(), 1, &mut len) };
    let short_len = len;
    let after_short = dispatch_together();
    let take = unsafe {
        api.context_take_command.unwrap()(context, out.as_mut_ptr(), out.len(), &mut len)
    };
    // SAFETY: all producer threads have joined and this test owns the context.
    unsafe { api.context_destroy.unwrap()(context) };

    assert_eq!(
        first
            .iter()
            .filter(|&&status| status == CaliberStatus::Ok)
            .count(),
        1
    );
    assert_eq!(
        first
            .iter()
            .filter(|&&status| status == CaliberStatus::QueueFull)
            .count(),
        PRODUCERS - 1
    );
    assert_eq!((peek, peek_len), (CaliberStatus::Ok, 2));
    assert!(
        after_peek
            .iter()
            .all(|&status| status == CaliberStatus::QueueFull)
    );
    assert_eq!((short, short_len), (CaliberStatus::BufferTooSmall, 2));
    assert!(
        after_short
            .iter()
            .all(|&status| status == CaliberStatus::QueueFull)
    );
    assert_eq!((take, len), (CaliberStatus::Ok, 2));
    assert_eq!(
        usize::from(out[0]),
        first
            .iter()
            .position(|&status| status == CaliberStatus::Ok)
            .unwrap()
    );
    assert_eq!(out[1], 0x5a);
}

#[test]
fn concurrent_c_abi_consumers_report_the_required_size_and_deliver_once() {
    const COMMANDS: u64 = 30_000;
    const SHORT: usize = 8;
    const LONG: usize = 64;
    const CONSUMERS: usize = 6;

    // SAFETY: version 1 has a static function table for the life of the process.
    let api = unsafe { &*caliber_get_api(CALIBER_ABI_VERSION_1) };
    let create = api.context_create.unwrap();
    let destroy = api.context_destroy.unwrap();
    let dispatch = api.context_dispatch.unwrap();
    let take = api.context_take_command.unwrap();
    let config = CaliberContextConfig {
        max_pending_commands: 32,
        ..CaliberContextConfig::default()
    };
    let mut context = ptr::null_mut();
    // SAFETY: config and the output pointer are valid for the call.
    assert_eq!(unsafe { create(&config, &mut context) }, CaliberStatus::Ok);
    assert!(!context.is_null());
    // Contexts support concurrent calls and remain live until all threads join.
    let address = context as usize;
    let stop = Arc::new(AtomicBool::new(false));
    let producer_done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(CONSUMERS + 1));

    let (producer_result, consumer_results) = thread::scope(|scope| {
        let producer = {
            let stop = Arc::clone(&stop);
            let producer_done = Arc::clone(&producer_done);
            let start = Arc::clone(&start);
            scope.spawn(move || -> Result<(), String> {
                start.wait();
                for id in 0..COMMANDS {
                    let mut command = [0xa5; LONG];
                    command[..SHORT].copy_from_slice(&id.to_le_bytes());
                    let len = if id % 2 == 0 { SHORT } else { LONG };
                    loop {
                        if stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        // SAFETY: context and command remain live throughout the call.
                        match unsafe {
                            dispatch(address as *const CaliberContext, command.as_ptr(), len)
                        } {
                            CaliberStatus::Ok => break,
                            CaliberStatus::QueueFull => thread::yield_now(),
                            status => {
                                stop.store(true, Ordering::Release);
                                return Err(format!("dispatch returned {status:?}"));
                            }
                        }
                    }
                }
                producer_done.store(true, Ordering::Release);
                Ok(())
            })
        };

        let consumers: Vec<_> = (0..CONSUMERS)
            .map(|index| {
                let stop = Arc::clone(&stop);
                let producer_done = Arc::clone(&producer_done);
                let start = Arc::clone(&start);
                scope.spawn(move || -> Result<Vec<u64>, String> {
                    let capacity = if index < 4 { SHORT } else { LONG };
                    let mut received = Vec::new();
                    start.wait();
                    while !stop.load(Ordering::Acquire) {
                        let mut out = [0u8; LONG];
                        let mut len = usize::MAX;
                        // SAFETY: context stays live, and each output belongs to this thread.
                        let status = unsafe {
                            take(address as *const CaliberContext, out.as_mut_ptr(), capacity, &mut len)
                        };
                        match status {
                            CaliberStatus::Ok => {
                                if (len != SHORT && len != LONG) || len > capacity {
                                    stop.store(true, Ordering::Release);
                                    return Err(format!("invalid successful length {len}"));
                                }
                                let id = u64::from_le_bytes(out[..SHORT].try_into().unwrap());
                                let expected = if id % 2 == 0 { SHORT } else { LONG };
                                if id >= COMMANDS || len != expected || out[SHORT..len].iter().any(|&b| b != 0xa5) {
                                    stop.store(true, Ordering::Release);
                                    return Err(format!("invalid command {id} with length {len}"));
                                }
                                received.push(id);
                            }
                            CaliberStatus::BufferTooSmall => {
                                if len != LONG || len <= capacity {
                                    stop.store(true, Ordering::Release);
                                    return Err(format!("short buffer capacity {capacity} reported required length {len}"));
                                }
                                thread::yield_now();
                            }
                            CaliberStatus::Unavailable => {
                                if len != usize::MAX {
                                    stop.store(true, Ordering::Release);
                                    return Err(format!("empty queue unexpectedly reported length {len}"));
                                }
                                if producer_done.load(Ordering::Acquire) {
                                    break;
                                }
                                thread::yield_now();
                            }
                            status => {
                                stop.store(true, Ordering::Release);
                                return Err(format!("take returned {status:?}"));
                            }
                        }
                    }
                    Ok(received)
                })
            })
            .collect();
        let producer_result = producer.join();
        let consumer_results: Vec<_> = consumers
            .into_iter()
            .map(|consumer| consumer.join())
            .collect();
        (producer_result, consumer_results)
    });

    // SAFETY: every producer and consumer has joined and context is still owned here.
    unsafe { destroy(context) };
    producer_result
        .expect("producer panicked")
        .expect("producer failed");
    let mut ids = Vec::new();
    for result in consumer_results {
        let received = result.expect("consumer panicked").expect("consumer failed");
        ids.extend(received);
    }
    ids.sort_unstable();
    assert_eq!(ids, (0..COMMANDS).collect::<Vec<_>>());
}

#[test]
fn c_client_capacity_one_counts_peeked_and_short_commands() {
    let api = caliber_get_api(CALIBER_ABI_VERSION_1);
    assert!(!api.is_null());
    // SAFETY: the API is Caliber's static v1 table; the C client owns its context.
    let result = unsafe { caliber_command_capacity_client_run(api) };
    assert_eq!(
        result, 0,
        "C queue client failed at command_queue.c:{result}"
    );
}
