use anyhow::{Result, anyhow};
use async_hid::{AccessMode, Device, DeviceInfo};
use futures::{StreamExt, future::ready};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::sync::mpsc::{self, Sender};
use tokio_util::sync::CancellationToken;

use crate::consts::{QMK_USAGE_ID, QMK_USAGE_PAGE};

pub struct KeyboardDevice {
    hid: Arc<Device>,
    listener: CancellationToken,
    reporter: Sender<(Vec<u8>, FutureReportState)>,
    win_lock: tokio::sync::Mutex<()>,
}

impl KeyboardDevice {
    pub async fn from_ids(vendor_id: u16, product_id: u16) -> Result<Self> {
        let hid = DeviceInfo::enumerate()
            .await?
            .filter(|info| ready(info.matches(QMK_USAGE_PAGE, QMK_USAGE_ID, vendor_id, product_id)))
            .next()
            .await
            .ok_or_else(|| {
                anyhow!(
                    "A device cannot be detected (VID: {}, PID: {})!",
                    vendor_id,
                    product_id
                )
            })?
            .open(AccessMode::ReadWrite)
            .await?;

        let hid = Arc::new(hid);
        let weak_hid = Arc::downgrade(&hid);

        let listener = CancellationToken::new();
        let signal = listener.clone();

        let (reporter, mut receiver) = mpsc::channel::<(Vec<u8>, FutureReportState)>(32);

        tokio::spawn(async move {
            let mut requests: Vec<(Vec<u8>, FutureReportState)> = Vec::new();

            loop {
                let hid = match weak_hid.upgrade() {
                    None => return,
                    Some(x) => x,
                };

                let mut buffer = [0u8; 32];
                tokio::select! {
                    _ = signal.cancelled() => { return; }

                    Some(request) = receiver.recv() => {
                        requests.push(request);
                    }

                    _ = hid.read_input_report(&mut buffer) => {
                        requests.retain(|x| {
                            if buffer.starts_with(&x.0) {
                                let mut state = x.1.lock().unwrap();
                                state.data = Some(buffer);
                                if let Some(waker) = state.waker.take() {
                                    waker.wake();
                                }
                                return false;
                            } else {
                                return true;
                            }
                        });
                    }
                }
            }
        });

        Ok(KeyboardDevice {
            hid,
            reporter,
            listener,
            win_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn send_report(&self, report: [u8; 32]) -> Result<()> {
        if cfg!(target_os = "windows") {
            let mut report_with_id = [0u8; 33];
            report_with_id[1..33].copy_from_slice(&report);

            let lock = self.win_lock.lock().await;
            self.hid
                .write_output_report(&report_with_id)
                .await
                .map_err(|err| anyhow::Error::from(err))?;
            drop(lock);
        } else {
            self.hid
                .write_output_report(&report)
                .await
                .map_err(|err| anyhow::Error::from(err))?;
        }

        Ok(())
    }

    pub async fn request_report(&self, report: [u8; 32], ref_bytes: usize) -> Result<[u8; 32]> {
        let prefix = report[..ref_bytes].to_vec();
        let state = Arc::new(Mutex::new(ReportFutureInner {
            data: None,
            waker: None,
        }));

        self.reporter.send((prefix, state.clone())).await?;
        self.send_report(report).await?;
        Ok(FutureReport { state }.await)
    }
}

impl Drop for KeyboardDevice {
    fn drop(&mut self) {
        self.listener.cancel();
    }
}

struct ReportFutureInner {
    data: Option<[u8; 32]>,
    waker: Option<Waker>,
}

type FutureReportState = Arc<Mutex<ReportFutureInner>>;

struct FutureReport {
    state: FutureReportState,
}

impl Future for FutureReport {
    type Output = [u8; 32];

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();

        if let Some(data) = state.data.take() {
            Poll::Ready(data)
        } else {
            state.waker = Some(ctx.waker().clone());
            Poll::Pending
        }
    }
}
