//! Self-contained SPP pairing on macOS.
//!
//! The N200 Pro uses Secure Simple Pairing with "just works" semantics (no
//! display or keypad on the printer). We drive `IOBluetoothDevicePair` on the
//! main queue, which is pumped by `platform::run`.

#[cfg(target_os = "macos")]
pub async fn pair_device(address: &str) -> anyhow::Result<()> {
    use std::cell::RefCell;

    use anyhow::{anyhow, bail};
    use futures::channel::oneshot;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{NSObjectProtocol, NSString};
    use objc2_io_bluetooth::{
        IOBluetoothDevice, IOBluetoothDevicePair, IOBluetoothDevicePairDelegate,
    };
    use waterkit_core::apple::on_main;

    struct PairIvars {
        tx: RefCell<Option<oneshot::Sender<Result<(), String>>>>,
        pairer: RefCell<Option<Retained<IOBluetoothDevicePair>>>,
        // Self-retain: IOBluetooth delegate properties are weak. The cycle is
        // intentionally leaked; pairing runs once and the process exits.
        keep_alive: RefCell<Option<Retained<PairDelegate>>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "LienePairDelegate"]
        #[ivars = PairIvars]
        struct PairDelegate;

        unsafe impl NSObjectProtocol for PairDelegate {}

        unsafe impl IOBluetoothDevicePairDelegate for PairDelegate {
            #[unsafe(method(devicePairingFinished:error:))]
            fn pairing_finished(&self, _sender: Option<&AnyObject>, error: i32) {
                if let Some(tx) = self.ivars().tx.borrow_mut().take() {
                    let _ = tx.send(if error == 0 {
                        Ok(())
                    } else {
                        Err(format!("pairing failed (IOReturn {error})"))
                    });
                }
            }

            #[unsafe(method(devicePairingUserConfirmationRequest:numericValue:))]
            fn confirm_numeric(&self, _sender: Option<&AnyObject>, numeric_value: u32) {
                eprintln!("# confirm numeric code {numeric_value:06} on both devices");
                if let Some(pairer) = self.ivars().pairer.borrow().as_ref() {
                    // SAFETY: replying to the numeric-comparison prompt.
                    unsafe { pairer.replyUserConfirmation(true) };
                }
            }

            #[unsafe(method(devicePairingUserPasskeyNotification:passkey:))]
            fn passkey(&self, _sender: Option<&AnyObject>, passkey: u32) {
                eprintln!("# enter passkey {passkey:06} on the printer if it has a keypad");
            }
        }
    );

    impl PairDelegate {
        fn new(mtm: MainThreadMarker, tx: oneshot::Sender<Result<(), String>>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(PairIvars {
                tx: RefCell::new(Some(tx)),
                pairer: RefCell::new(None),
                keep_alive: RefCell::new(None),
            });
            // SAFETY: standard NSObject initialization for a define_class! type.
            unsafe { msg_send![super(this), init] }
        }
    }

    let address = address.to_owned();
    let (tx, rx) = oneshot::channel();

    on_main(move |mtm| {
        let ns_address = NSString::from_str(&address);
        // SAFETY: returns nil for an unknown address.
        let Some(device) =
            (unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&ns_address)) })
        else {
            let _ = tx.send(Err(format!("no Bluetooth device at {address}")));
            return;
        };

        // SAFETY: read-only pairing state.
        if unsafe { device.isPaired() } {
            eprintln!("# {address} is already paired");
            let _ = tx.send(Ok(()));
            return;
        }

        let delegate = PairDelegate::new(mtm, tx);
        // SAFETY: creates a pairing object for the target device.
        let Some(pairer) = (unsafe { IOBluetoothDevicePair::pairWithDevice(Some(&device)) }) else {
            let _ = delegate.ivars().tx.borrow_mut().take().map(|tx| {
                let _ = tx.send(Err("IOBluetoothDevicePair creation failed".to_owned()));
            });
            return;
        };

        // SAFETY: setDelegate stores a weak reference; `keep_alive` below
        // keeps the delegate (and this pairer) alive across callbacks.
        unsafe { pairer.setDelegate(Some(&**delegate)) };

        *delegate.ivars().pairer.borrow_mut() = Some(pairer.clone());
        *delegate.ivars().keep_alive.borrow_mut() = Some(delegate.clone());

        // SAFETY: starts the pairing attempt.
        let status = unsafe { pairer.start() };
        if status != 0 {
            if let Some(tx) = delegate.ivars().tx.borrow_mut().take() {
                let _ = tx.send(Err(format!("pairing start failed (IOReturn {status})")));
            }
        }
    })
    .await;

    match rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => bail!("{message}"),
        Err(_) => Err(anyhow!("pairing callback was dropped")),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn pair_device(_address: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    anyhow::bail!(
        "pairing is handled by BlueZ. Run `bluetoothctl`, then `pair <MAC>`, `trust <MAC>`, \
         and `connect <MAC>` before using liene-n200."
    );
    #[cfg(target_os = "windows")]
    anyhow::bail!(
        "pair the printer in Windows Settings > Bluetooth & devices > Add device, then retry."
    );
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    anyhow::bail!("pairing is not implemented on this platform; use the OS Bluetooth settings.");
}
