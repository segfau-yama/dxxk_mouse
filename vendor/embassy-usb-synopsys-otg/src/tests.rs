//! Poll the actual driver against RAM registers. W1C and self-clearing hardware
//! bits are NOT generally emulated. Selected tests opt into a small NAK/EPDIS/
//! flush model to exercise successful recovery as well as fail-closed paths.
//! This is not a silicon model or a test of USB bus timing.
use super::*;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Waker},
};
use embassy_usb_driver::Bus as _;
use std::{cell::Cell, sync::Arc, task::Wake};

std::thread_local! {
    static IN_HANDSHAKE: Cell<Option<(Otg, bool)>> = const { Cell::new(None) };
}

// The caller owns the RAM registers throughout `f`; the scoped guard also
// clears the pointer on panic. Separate test threads never share this model.
fn with_in_handshake(r: Otg, flush_succeeds: bool, f: impl FnOnce()) {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            IN_HANDSHAKE.set(None);
        }
    }
    assert!(IN_HANDSHAKE.get().is_none());
    IN_HANDSHAKE.set(Some((r, flush_succeeds)));
    let _clear = Clear;
    f();
}

pub(super) fn step_in_handshake() {
    let Some((r, flush_succeeds)) = IN_HANDSHAKE.get() else {
        return;
    };
    r.diepctl(1).modify(|w| {
        if w.snak() {
            w.set_naksts(true);
            w.set_snak(false);
        }
        if w.epdis() && w.naksts() {
            w.set_epena(false);
            w.set_epdis(false);
            // Model a lost TSIZ on abort: recovery must NOT reuse it.
            r.dieptsiz(1).write(|_| {});
        }
    });
    if r.grstctl().read().txfflsh() && flush_succeeds {
        assert_eq!(r.grstctl().read().txfnum(), 1, "flush only the microphone FIFO");
        assert!(!r.diepctl(1).read().epena(), "stop before flushing");
        unsafe { r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(25)) };
        r.fifo(1).write_value(regs::Fifo(0));
        r.grstctl().modify(|w| w.set_txfflsh(false));
    }
}

#[derive(Default)]
struct WakeFlag(AtomicBool);
impl Wake for WakeFlag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
struct TestMutex(PhantomData<*mut ()>);
// Each fixture stays on one test thread; the !Sync marker prevents sharing it.
unsafe impl RawMutex for TestMutex {
    const INIT: Self = Self(PhantomData);
    fn lock<R>(&self, f: impl FnOnce() -> R) -> R {
        f()
    }
}

fn fixture(storage: &StateStorage<2, TestMutex>, r: Otg) -> (Bus<'_, TestMutex>, Endpoint<'_, In, TestMutex>) {
    let st = storage.as_state();
    unsafe {
        st.alloc_slot_write(
            Direction::In,
            1,
            EndpointData {
                ep_type: EndpointType::Isochronous,
                max_packet_size: 98,
                fifo_size_words: 25,
                tx_fifo: 1,
            },
        );
        r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(25));
    }
    st.ep_states[1].in_enabled.store(true, Ordering::Release);
    r.diepctl(1).write(|w| {
        w.set_usbaep(true);
        w.set_eptyp(vals::Eptyp::ISOCHRONOUS);
        w.set_mpsiz(98);
        w.set_txfnum(1);
    });
    let bus = Bus {
        config: Config::default(),
        inited: true,
        fifo_layout_ready: true,
        instance: OtgInstance {
            regs: r,
            state: st,
            fifo_depth_words: 256,
            extra_rx_fifo_words: 16,
            phy_type: PhyType::InternalFullSpeed,
            tx_fifo_count: 2,
            calculate_trdt_fn: |_| 5,
        },
    };
    let endpoint = Endpoint {
        _phantom: PhantomData,
        regs: r,
        state: &storage.ep_states[1],
        mutex: TestMutex::INIT,
        info: EndpointInfo {
            addr: EndpointAddress::from_parts(1, Direction::In),
            ep_type: EndpointType::Isochronous,
            max_packet_size: 98,
            interval_ms: 1,
        },
    };
    (bus, endpoint)
}

#[test]
fn old_write_is_cancelled_in_both_waits_even_after_reenable_and_generation_wrap() {
    for phase in [2, 3] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, mut endpoint) = fixture(&storage, r);
        let ep = &storage.ep_states[1];
        ep.in_generation.store(u32::MAX, Ordering::Release);
        if phase == 2 {
            r.diepctl(1).modify(|w| w.set_epena(true));
        } else {
            // Not the orphaned EPENA=0 / TSIZ=0 case: keep normal FIFO waiting.
            r.dieptsiz(1).write(|w| w.set_pktcnt(1));
            unsafe {
                r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(0));
            }
        }
        let mut write = pin!(endpoint.write(&[42; 96]));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(write.as_mut().poll(&mut cx).is_pending());
        assert_eq!(ep.in_phase.load(Ordering::Relaxed), phase);
        ep.invalidate_in(); // reset / alt change while the future is asleep
        ep.in_enabled.store(true, Ordering::Release);
        r.diepctl(1).modify(|w| w.set_epena(false));
        r.dieptsiz(1).write(|_| {});
        unsafe {
            r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(25));
        }
        assert_eq!(write.as_mut().poll(&mut cx), Poll::Ready(Err(EndpointError::Disabled)));
        assert_eq!(ep.in_generation.load(Ordering::Acquire), 0);
        assert_eq!(ep.in_started.load(Ordering::Relaxed), 1);
        assert_eq!(ep.in_cancelled.load(Ordering::Relaxed), 1);
        assert_eq!(ep.in_queued.load(Ordering::Relaxed), 0);
        assert_eq!(r.dieptsiz(1).read().0, 0);
    }
}

#[test]
fn writes_are_reservations_and_only_xfrc_interrupts_count_completions() {
    let mut ram = Box::new([0u32; 4096]);
    let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
    let storage = StateStorage::new(TestMutex::INIT);
    let (_, mut endpoint) = fixture(&storage, r);
    let st = storage.as_state();
    for size in [94, 96, 98] {
        let data = [0x5a; 98];
        let mut write = pin!(endpoint.write(&data[..size]));
        assert_eq!(
            write.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        );
        assert_eq!(r.dieptsiz(1).read().xfrsiz(), size as u32);
        r.diepctl(1).modify(|w| w.set_epena(false)); // emulate completed packet
    }
    assert_eq!(st.in_transfer_counts(1), Some((3, 0)));
    r.gintsts().write(|w| w.set_iepint(true));
    unsafe {
        r.daint().as_ptr().write(regs::Daint(2));
    }
    for xfrc in [false, true] {
        r.diepint(1).write(|w| {
            w.set_txfe(true);
            w.set_xfrc(xfrc);
        });
        unsafe {
            on_interrupt(r, &st);
        }
        assert_eq!(st.in_transfer_counts(1), Some((3, u32::from(xfrc))));
    }
    let before = ram.clone();
    let snapshot = unsafe { st.in_transfer_snapshot(r, 1) }.unwrap();
    assert_eq!(&snapshot[..2], &[3, 1]);
    assert_eq!(snapshot[2], 3);
    assert_eq!(snapshot[12], r.diepctl(1).read().0);
    assert_eq!(ram, before); // read-only: no interrupt acknowledgement
    assert!(unsafe { st.in_transfer_snapshot(r, 0) }.is_none());
    assert!(unsafe { st.in_transfer_snapshot(r, 2) }.is_none());
}

#[test]
fn repeated_alt1_aborts_before_flush_and_does_not_accept_stale_epdisd() {
    for nak_effective in [false, true] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (mut bus, _) = fixture(&storage, r);
        r.diepctl(1).modify(|w| {
            w.set_epena(true);
            w.set_naksts(nak_effective);
        });
        r.diepint(1).write(|w| w.set_epdisd(true)); // stale, EPENA is still 1
        r.diepempmsk().write(|w| w.set_ineptxfem(0b110));
        bus.endpoint_set_enabled(EndpointAddress::from_parts(1, Direction::In), true);
        let ep = &storage.ep_states[1];
        assert!(!ep.in_enabled.load(Ordering::Acquire));
        assert!(!r.diepctl(1).read().usbaep());
        assert_eq!(r.grstctl().read().0, 0, "must not flush an active transfer");
        assert_eq!(r.diepempmsk().read().ineptxfem(), 0b100, "preserve other endpoints");
        assert_eq!(
            ep.in_last_error.load(Ordering::Relaxed),
            if nak_effective { 2 } else { 1 }
        );
    }
}

#[test]
fn flush_failure_is_not_published_as_enabled() {
    let mut ram = Box::new([0u32; 4096]);
    let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
    let storage = StateStorage::new(TestMutex::INIT);
    let (mut bus, _) = fixture(&storage, r);
    // EPENA=0, but RAM does not self-clear TXFFLSH: emulate flush timeout.
    bus.endpoint_set_enabled(EndpointAddress::from_parts(1, Direction::In), true);
    let ep = &storage.ep_states[1];
    assert!(!ep.in_enabled.load(Ordering::Acquire));
    assert_eq!(ep.in_flush_timeouts.load(Ordering::Relaxed), 1);
    assert_eq!(ep.in_last_error.load(Ordering::Relaxed), 3);
    let busy_command = r.grstctl().read().0;
    assert_eq!(flush_tx_fifo(r, 0), Err(InStopError::Flush));
    flush_rx_fifo(r);
    assert_eq!(
        r.grstctl().read().0,
        busy_command,
        "do not replace an unfinished flush command"
    );
}

#[test]
fn incomplete_iso_does_not_resurrect_disabled_or_reset_endpoints() {
    for reset in [false, true] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, _) = fixture(&storage, r);
        let ep = &storage.ep_states[1];
        ep.in_enabled.store(reset, Ordering::Release);
        r.diepctl(1).modify(|w| w.set_epena(true));
        r.gintsts().write(|w| {
            w.set_iisoixfr(true);
            w.set_usbrst(reset);
        });
        unsafe {
            on_interrupt(r, &storage.as_state());
        }
        assert_eq!(ep.in_incomplete.load(Ordering::Relaxed), 0);
        assert!(!r.diepctl(1).read().cnak());
    }
}

#[test]
fn incomplete_iso_abort_failure_does_not_rearm() {
    let mut ram = Box::new([0u32; 4096]);
    let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
    let storage = StateStorage::new(TestMutex::INIT);
    let (_, _) = fixture(&storage, r);
    r.diepctl(1).modify(|w| w.set_epena(true));
    r.gintsts().write(|w| w.set_iisoixfr(true));
    unsafe {
        on_interrupt(r, &storage.as_state());
    }
    let ep = &storage.ep_states[1];
    assert_eq!(ep.in_incomplete.load(Ordering::Relaxed), 1);
    assert_eq!(ep.in_last_error.load(Ordering::Relaxed), 1);
    assert!(!ep.in_enabled.load(Ordering::Acquire));
    assert!(!r.diepctl(1).read().cnak());
}

#[test]
fn idle_iso_fifo_from_reported_snapshot_is_reclaimed_before_next_packet() {
    for size in [94, 96, 98] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, mut endpoint) = fixture(&storage, r);
        // 22:18 log: EPENA=0, TSIZ=0, just five words free, TXFE mask set.
        r.diepctl(1).write_value(regs::Diepctl(0x00448062));
        r.diepempmsk().write(|w| w.set_ineptxfem(0b110));
        unsafe { r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(5)) };
        r.fifo(0).write_value(regs::Fifo(0xdeadbeef));
        let data = [0x5a; 98];
        let mut write = pin!(endpoint.write(&data[..size]));
        with_in_handshake(r, true, || {
            assert_eq!(
                write.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Ok(()))
            );
        });
        assert_eq!(r.dieptsiz(1).read().xfrsiz(), size as u32);
        assert_eq!(r.dieptsiz(1).read().pktcnt(), 1);
        assert!(r.diepctl(1).read().epena());
        assert_eq!(r.fifo(0).read().0, 0xdeadbeef, "EP0 data stays intact");
        assert_eq!(r.diepempmsk().read().ineptxfem(), 0b100);
        assert_eq!(storage.as_state().in_transfer_counts(1), Some((1, 0)));
        assert_eq!(storage.ep_states[1].in_generation.load(Ordering::Relaxed), 0);
        assert_eq!(storage.ep_states[1].in_cancelled.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn expired_iso_packet_is_flushed_and_wakes_next_write_without_rearming_old_tsiz() {
    for odd_frame in [false, true] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, mut endpoint) = fixture(&storage, r);
        assert_eq!(
            pin!(endpoint.write(&[0x33; 96]))
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        );
        let flag = Arc::new(WakeFlag::default());
        let waker = Waker::from(flag.clone());
        let mut cx = Context::from_waker(&waker);
        let mut next_write = pin!(endpoint.write(&[0x55; 98]));
        assert!(next_write.as_mut().poll(&mut cx).is_pending());
        r.diepctl(1).modify(|w| {
            // These frame-selection command bits self-clear on hardware.
            w.set_sd0pid_sevnfrm(false);
            w.set_soddfrm_sd1pid(false);
            w.set_cnak(false);
            w.set_eonum_dpid(odd_frame);
        });
        unsafe {
            r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(5));
            r.dsts().as_ptr().write(regs::Dsts(u32::from(odd_frame) << 8));
        }
        r.gintsts().write(|w| w.set_iisoixfr(true));
        with_in_handshake(r, true, || unsafe { on_interrupt(r, &storage.as_state()) });
        let ep = &storage.ep_states[1];
        assert!(flag.0.load(Ordering::Relaxed), "next write must be woken without XFRC");
        assert!(!r.diepctl(1).read().epena(), "never rearm the discarded packet");
        assert!(ep.in_enabled.load(Ordering::Acquire));
        assert!(r.diepctl(1).read().usbaep());
        assert_eq!(r.dieptsiz(1).read().0, 0);
        assert_eq!(r.dtxfsts(1).read().ineptfsav(), 25);
        assert_eq!(ep.in_incomplete.load(Ordering::Relaxed), 1);
        assert_eq!(storage.as_state().in_transfer_counts(1), Some((1, 0)));
        assert_eq!(next_write.as_mut().poll(&mut cx), Poll::Ready(Ok(())));
        assert_eq!(r.dieptsiz(1).read().xfrsiz(), 98);
        assert_eq!(r.dieptsiz(1).read().pktcnt(), 1);
        assert_eq!(r.diepctl(1).read().sd0pid_sevnfrm(), odd_frame);
        assert_eq!(r.diepctl(1).read().soddfrm_sd1pid(), !odd_frame);
        assert_eq!(storage.as_state().in_transfer_counts(1), Some((2, 0)));
        assert_eq!(ep.in_generation.load(Ordering::Relaxed), 0);
        assert_eq!(ep.in_cancelled.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn iso_cleanup_flush_failure_returns_disabled_instead_of_waiting_forever() {
    for incomplete_irq in [false, true] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, mut endpoint) = fixture(&storage, r);
        r.diepctl(1).modify(|w| w.set_epena(incomplete_irq));
        unsafe { r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(5)) };
        let mut write = pin!(endpoint.write(&[0; 96]));
        let mut cx = Context::from_waker(Waker::noop());
        with_in_handshake(r, false, || {
            if incomplete_irq {
                assert!(write.as_mut().poll(&mut cx).is_pending());
                r.gintsts().write(|w| w.set_iisoixfr(true));
                unsafe { on_interrupt(r, &storage.as_state()) };
            }
            assert_eq!(write.as_mut().poll(&mut cx), Poll::Ready(Err(EndpointError::Disabled)));
        });
        let ep = &storage.ep_states[1];
        assert_eq!(ep.in_flush_timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(ep.in_last_error.load(Ordering::Relaxed), 3);
        assert_eq!(ep.in_cancelled.load(Ordering::Relaxed), 1);
        assert!(!ep.in_enabled.load(Ordering::Acquire));
        assert!(!r.diepctl(1).read().epena());
        assert_eq!(storage.as_state().in_transfer_counts(1), Some((0, 0)));
    }
}

#[test]
fn fifo_reclaim_does_not_flush_live_or_non_iso_transfers() {
    for (ep_type, active, remaining) in [
        (EndpointType::Isochronous, true, false),
        (EndpointType::Isochronous, false, true),
        (EndpointType::Bulk, false, false),
        (EndpointType::Interrupt, false, false),
        (EndpointType::Control, false, false),
    ] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (_, mut endpoint) = fixture(&storage, r);
        endpoint.info.ep_type = ep_type;
        r.diepctl(1).modify(|w| w.set_epena(active));
        r.dieptsiz(1).write(|w| w.set_xfrsiz(u32::from(remaining) * 96));
        unsafe { r.dtxfsts(1).as_ptr().write(regs::Dtxfsts(5)) };
        let mut write = pin!(endpoint.write(&[0; 96]));
        assert!(
            write
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(r.grstctl().read().0, 0);
        assert!(storage.ep_states[1].in_enabled.load(Ordering::Acquire));
    }
}

#[test]
fn reset_invalidates_writes_before_any_fifo_flush() {
    let mut ram = Box::new([0u32; 4096]);
    let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
    let storage = StateStorage::new(TestMutex::INIT);
    let (mut bus, _) = fixture(&storage, r);
    bus.fifo_layout_ready = false; // first controller initialization
    r.diepctl(1).modify(|w| w.set_epena(true));
    bus.init_fifo();
    assert!(!bus.fifo_layout_ready);
    let ep = &storage.ep_states[1];
    assert!(!ep.in_enabled.load(Ordering::Acquire));
    assert_ne!(ep.in_generation.load(Ordering::Acquire), 0);
    assert_eq!(r.grstctl().read().0, 0);
    bus.endpoint_set_enabled(EndpointAddress::from_parts(1, Direction::In), true);
    assert!(!ep.in_enabled.load(Ordering::Acquire));
}

#[test]
fn power_removal_and_deinit_cancel_sleeping_writes() {
    for power_removed in [false, true] {
        let mut ram = Box::new([0u32; 4096]);
        let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
        let storage = StateStorage::new(TestMutex::INIT);
        let (mut bus, mut endpoint) = fixture(&storage, r);
        r.diepctl(1).modify(|w| w.set_epena(true));
        let mut write = pin!(endpoint.write(&[0; 96]));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(write.as_mut().poll(&mut cx).is_pending());
        if power_removed {
            bus.disable_all_endpoints();
        } else {
            bus.deinit_device();
        }
        assert_eq!(write.as_mut().poll(&mut cx), Poll::Ready(Err(EndpointError::Disabled)));
        assert_eq!(storage.as_state().in_transfer_counts(1), Some((0, 0)));
    }
}

#[test]
fn reset_configuration_keeps_ep0_usable_when_mic_stop_failed() {
    let mut ram = Box::new([0u32; 4096]);
    let r = unsafe { Otg::from_ptr(ram.as_mut_ptr().cast()) };
    let storage = StateStorage::new(TestMutex::INIT);
    let (mut bus, _) = fixture(&storage, r);
    unsafe {
        storage.as_state().alloc_slot_write(
            Direction::In,
            0,
            EndpointData {
                ep_type: EndpointType::Control,
                max_packet_size: 64,
                fifo_size_words: 16,
                tx_fifo: 0,
            },
        );
        r.dtxfsts(0).as_ptr().write(regs::Dtxfsts(16));
    }
    r.diepctl(1).modify(|w| w.set_epena(true));
    fail_in_endpoint(r, 1, &storage.ep_states[1], InStopError::Nak);
    let failed_ctl = r.diepctl(1).read();
    // Supply the result of a successful EP0 stop/flush and failed EP1 stop.
    // Successful hardware handshakes themselves are not emulated here.
    bus.configure_endpoints(0b01);
    assert!(storage.ep_states[0].in_enabled.load(Ordering::Acquire));
    assert!(!storage.ep_states[1].in_enabled.load(Ordering::Acquire));
    assert_eq!(r.diepctl(1).read().0, failed_ctl.0);
    let mut control = Endpoint::<In, _> {
        _phantom: PhantomData,
        regs: r,
        state: &storage.ep_states[0],
        mutex: TestMutex::INIT,
        info: EndpointInfo {
            addr: EndpointAddress::from_parts(0, Direction::In),
            ep_type: EndpointType::Control,
            max_packet_size: 64,
            interval_ms: 0,
        },
    };
    assert_eq!(
        pin!(control.write(&[0; 32]))
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    );
}
