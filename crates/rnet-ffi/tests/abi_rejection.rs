use rnet::*;

#[test]
fn only_the_current_abi_and_exact_layout_are_accepted() {
    assert_ne!(
        rnet_abi_version(),
        1,
        "old SDK layouts must not share the current ABI"
    );
    for (abi, size) in [
        (0, std::mem::size_of::<RnetConfig>() as u32),
        (1, std::mem::size_of::<RnetConfig>() as u32),
        (
            RNET_ABI_VERSION + 1,
            std::mem::size_of::<RnetConfig>() as u32,
        ),
        (
            RNET_ABI_VERSION,
            std::mem::size_of::<RnetConfig>() as u32 + 8,
        ),
    ] {
        let config = RnetConfig {
            abi_version: abi,
            struct_size: size,
            ..RnetConfig::default()
        };
        let mut output = 0xdead;
        assert_eq!(
            unsafe { rnet_runtime_create(&config, std::ptr::null(), &mut output) },
            RNET_E_INVALID_ARGUMENT
        );
        assert_eq!(output, 0xdead, "failed creation must not publish a runtime");
    }
}

// A protected next page makes an eager full-struct copy fail deterministically, even without
// ASan. Only the common header is readable; mismatched layouts must return before reading more.
#[cfg(target_os = "linux")]
#[test]
fn short_foreign_headers_are_rejected_without_reading_past_the_header() {
    struct Pages {
        base: *mut libc::c_void,
        len: usize,
    }
    impl Drop for Pages {
        fn drop(&mut self) {
            assert_eq!(unsafe { libc::munmap(self.base, self.len) }, 0);
        }
    }
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    assert!(page > 8);
    let page = page as usize;
    let base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            page * 2,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert_ne!(base, libc::MAP_FAILED);
    let pages = Pages {
        base,
        len: page * 2,
    };
    assert_eq!(
        unsafe { libc::mprotect(base.cast::<u8>().add(page).cast(), page, libc::PROT_NONE) },
        0
    );
    let header = unsafe { base.cast::<u8>().add(page - 8).cast::<u32>() };
    unsafe {
        header.write(8);
        header.add(1).write(RNET_ABI_VERSION);
    }
    let mut output = 0;
    assert_eq!(
        unsafe { rnet_runtime_create(header.cast(), std::ptr::null(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { rnet_game_runtime_create(header.cast(), std::ptr::null(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { rnet_runtime_create(&RnetConfig::default(), header.cast(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { rnet_game_runtime_create(&RnetGameConfig::default(), header.cast(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    let config = RnetConfig {
        logger: header.cast(),
        ..RnetConfig::default()
    };
    assert_eq!(
        unsafe { rnet_runtime_create(&config, std::ptr::null(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    let game = RnetGameConfig {
        logger: header.cast(),
        ..RnetGameConfig::default()
    };
    assert_eq!(
        unsafe { rnet_game_runtime_create(&game, std::ptr::null(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    let game = RnetGameConfig {
        network_config: header.cast(),
        ..RnetGameConfig::default()
    };
    assert_eq!(
        unsafe { rnet_game_runtime_create(&game, std::ptr::null(), &mut output) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { rnet_game_send_ex(0, 0, RnetSlice::default(), header.cast()) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(output, 0);
    drop(pages);
}
