use super::*;

fn engine() -> Kcp<Vec<u8>> {
    let mut kcp = Kcp::new(19, Vec::new());
    kcp.set_nodelay(true, 10, 2, true);
    kcp
}

#[test]
fn clock_differences_obey_half_space_order() {
    assert_eq!(timediff(0x8000_0010, 0x7fff_fff0), 32);
    assert_eq!(timediff(16, 0xffff_fff0), 32);
    assert_eq!(timediff(0xffff_fff0, 16), -32);
}

#[test]
fn selective_ack_finds_wrapped_serial_before_cumulative_ack_advances() {
    let mut kcp = engine();
    kcp.snd_una = u32::MAX;
    kcp.snd_nxt = 2;
    for sn in [u32::MAX, 0, 1] {
        kcp.snd_buf.push_back(KcpSegment {
            sn,
            ..Default::default()
        });
    }
    kcp.parse_ack(0);
    assert_eq!(
        kcp.snd_buf.iter().map(|s| s.sn).collect::<Vec<_>>(),
        [u32::MAX, 1]
    );
    kcp.parse_una(1);
    assert_eq!(kcp.snd_buf.front().unwrap().sn, 1);
}

#[test]
fn fragmented_delivery_and_ack_across_sequence_and_clock_wrap() {
    for start in [0x7fff_fffe, 0xffff_fffe] {
        let mut sender = engine();
        let mut receiver = engine();
        sender.snd_una = start;
        sender.snd_nxt = start;
        receiver.rcv_nxt = start;
        let payload: Vec<_> = (0..5000).map(|n| (n % 251) as u8).collect();
        sender.send(&payload).unwrap();
        let mut dropped = false;
        let mut delivered = false;
        for elapsed in (0..3000_u32).step_by(10) {
            let now = (u32::MAX - 20).wrapping_add(elapsed);
            sender.update(now).unwrap();
            let packet = std::mem::take(&mut sender.output.0);
            // The output is a byte stream of valid KCP segments. Reorder them
            // and drop one once; do not bypass input/fragment assembly/ACK.
            let mut segments = Vec::new();
            let mut offset = 0;
            while offset < packet.len() {
                let len = u32::from_le_bytes(packet[offset + 20..offset + 24].try_into().unwrap())
                    as usize;
                let end = offset + KCP_OVERHEAD + len;
                segments.push(&packet[offset..end]);
                offset = end;
            }
            for segment in segments.into_iter().rev() {
                if !dropped {
                    dropped = true;
                    continue;
                }
                receiver.input(segment).unwrap();
            }
            receiver.update(now).unwrap();
            let ack = std::mem::take(&mut receiver.output.0);
            if !ack.is_empty() {
                sender.input(&ack).unwrap();
            }
            let mut message = vec![0; payload.len()];
            match receiver.recv(&mut message) {
                Ok(size) => {
                    assert!(!delivered);
                    assert_eq!(size, payload.len());
                    assert_eq!(message, payload);
                    delivered = true;
                }
                Err(Error::RecvQueueEmpty | Error::ExpectingFragment) => {}
                Err(error) => panic!("{error}"),
            }
        }
        assert!(delivered);
        assert_eq!(sender.wait_snd(), 0, "all fragments must be acknowledged");
        assert_eq!(sender.snd_nxt, start.wrapping_add(4));
        assert_eq!(receiver.rcv_nxt, sender.snd_nxt);
    }
}

#[test]
fn invalid_mtu_does_not_mutate_configuration() {
    let mut kcp = engine();
    let mtu = kcp.mtu();
    for invalid in [0, 49, usize::MAX] {
        assert!(matches!(kcp.set_mtu(invalid), Err(Error::InvalidMtu(_))));
        assert_eq!(kcp.mtu(), mtu);
    }
}

#[test]
fn maximum_fragment_is_incomplete_not_integer_overflow() {
    let mut kcp = engine();
    kcp.rcv_queue.push_back(KcpSegment {
        frg: u8::MAX,
        ..Default::default()
    });
    assert!(matches!(kcp.peeksize(), Err(Error::ExpectingFragment)));
}

#[test]
fn large_rtt_and_retry_counters_do_not_wrap() {
    let mut kcp = engine();
    kcp.update_ack(i32::MAX as u32);
    kcp.update_ack(i32::MAX as u32);
    assert_eq!(kcp.rx_rto, KCP_RTO_MAX);
    kcp.snd_una = 0;
    kcp.snd_nxt = 2;
    kcp.snd_buf.push_back(KcpSegment {
        sn: 0,
        fastack: u32::MAX,
        ..Default::default()
    });
    kcp.parse_fastack(1, 0);
    assert_eq!(kcp.snd_buf[0].fastack, u32::MAX);
}

#[test]
fn zero_window_probe_and_retry_limits_survive_clock_wrap() {
    let mut kcp = engine();
    kcp.rmt_wnd = 0;
    kcp.update(u32::MAX - 10).unwrap();
    assert_eq!(kcp.ts_probe, 6989);
    kcp.update(6990).unwrap();
    assert_eq!(kcp.output.0[4], KCP_CMD_WASK);
    assert!(kcp.check(6990) <= 10);

    for nodelay in [false, true] {
        let mut kcp = engine();
        kcp.set_nodelay(nodelay, 10, 2, true);
        kcp.send(b"retry").unwrap();
        kcp.update(0).unwrap();
        kcp.xmit = u32::MAX;
        let segment = kcp.snd_buf.front_mut().unwrap();
        segment.xmit = u32::MAX;
        segment.rto = u32::MAX;
        segment.resendts = 10;
        kcp.update(10).unwrap();
        let segment = kcp.snd_buf.front().unwrap();
        assert_eq!(segment.xmit, u32::MAX);
        assert_eq!(kcp.xmit, u32::MAX);
        assert_eq!(segment.rto, KCP_RTO_MAX);
        assert_eq!(segment.resendts, 60010);
        assert!(kcp.is_dead_link());
    }
}

#[test]
fn ordinary_wire_header_is_unchanged() {
    let mut kcp = engine();
    kcp.send(b"abc").unwrap();
    kcp.update(0).unwrap();
    assert_eq!(
        kcp.output.0,
        [
            19, 0, 0, 0, 81, 0, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, b'a', b'b',
            b'c',
        ]
    );
}

#[test]
fn fast_retransmit_counts_wrapped_inflight_without_wrapping_the_window() {
    for threshold in [2, u32::MAX] {
        let mut kcp = engine();
        kcp.set_fast_resend(threshold);
        kcp.snd_una = u32::MAX;
        kcp.snd_nxt = 0;
        kcp.snd_buf.push_back(KcpSegment {
            sn: u32::MAX,
            xmit: 1,
            fastack: threshold,
            resendts: 100,
            rto: 200,
            ..Default::default()
        });
        kcp.update(0).unwrap();
        assert_eq!(kcp.snd_buf[0].xmit, 2);
        assert_eq!(kcp.cwnd, if threshold == 2 { 4 } else { u16::MAX });
        assert_eq!(kcp.snd_buf[0].resendts, 200);
    }
}

#[cfg(feature = "tokio")]
#[test]
fn async_update_uses_the_same_wrapping_contract() {
    use std::future::Future;
    use std::task::Waker;

    let mut context = Context::from_waker(Waker::noop());
    // Vec<u8>'s AsyncWrite completes immediately; all protocol work is real.
    let mut kcp = engine();
    kcp.snd_una = u32::MAX;
    kcp.snd_nxt = u32::MAX;
    kcp.send(b"async").unwrap();
    for now in [u32::MAX - 10, 20, 300] {
        let mut future = std::pin::pin!(kcp.async_update(now));
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Ok(()))
        ));
    }
    assert_eq!(kcp.snd_nxt, 0);
    assert!(kcp.snd_buf.front().unwrap().xmit > 1);
    assert_eq!(&kcp.output.0[12..16], &u32::MAX.to_le_bytes());
}
