use bytes::BytesMut;
use rnet_transport::{KcpEngine, RustKcpEngine};

// Real fragmented traffic, loss, reordering, ACKs and retransmission straddle
// both signed and unsigned clock boundaries in a downstream checked build.
#[test]
fn fragmented_delivery_across_clock_boundaries() {
    for start in [0x7fff_fff0_u64, 0xffff_fff0] {
        let mut sender = RustKcpEngine::new(17).unwrap();
        let mut receiver = RustKcpEngine::new(17).unwrap();
        let payload: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        sender.send(&payload).unwrap();
        let mut dropped = false;
        let mut delivered = None;
        for elapsed in (0..3000).step_by(10) {
            let now = start + elapsed;
            let mut outgoing = Vec::new();
            sender.update(now, &mut |p| outgoing.push(p.to_vec()));
            outgoing.reverse();
            for packet in outgoing {
                if !dropped {
                    dropped = true;
                    continue;
                }
                receiver.input(&packet, now).unwrap();
            }
            let mut acks = Vec::new();
            receiver.update(now, &mut |p| acks.push(p.to_vec()));
            for ack in acks {
                sender.input(&ack, now).unwrap();
            }
            let mut message = BytesMut::new();
            if receiver.recv(&mut message).unwrap().is_some() {
                assert!(delivered.is_none(), "message delivered twice");
                delivered = Some(message.to_vec());
            }
            assert!(sender.next_update_ms() >= now);
        }
        assert!(dropped);
        assert_eq!(delivered.as_deref(), Some(payload.as_slice()));
        assert!(sender.observed_rtt().is_some(), "ACK path must execute");
    }
}
