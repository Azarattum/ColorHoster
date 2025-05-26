use anyhow::{Result, anyhow};
use async_hid::{AsyncHidRead, AsyncHidWrite, DeviceWriter, HidBackend};
use futures::{StreamExt, future::ready};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tokio_util::sync::CancellationToken;

use crate::consts::{QMK_USAGE_ID, QMK_USAGE_PAGE};

type ReportRequest = (Vec<u8>, FutureReportState, oneshot::Sender<()>);

pub struct KeyboardDevice {
    writer: Arc<tokio::sync::Mutex<DeviceWriter>>,
    listener: CancellationToken,
    reporter: Sender<ReportRequest>,
}

impl KeyboardDevice {
    pub async fn from_ids(vendor_id: u16, product_id: u16) -> Result<Self> {
        let (mut reader, writer) = HidBackend::default()
            .enumerate()
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
            .open()
            .await?;

        let listener = CancellationToken::new();
        let signal = listener.clone();

        let (reporter, mut receiver) = mpsc::channel::<ReportRequest>(32);

        tokio::spawn(async move {
            let mut requests: Vec<(Vec<u8>, FutureReportState)> = Vec::new();

            loop {
                let mut buffer = [0u8; 32];
                tokio::select! {
                    _ = signal.cancelled() => { return; }

                    Some(request) = receiver.recv() => {
                        requests.push((request.0, request.1));
                        _ = request.2.send(());
                    }

                    _ = reader.read_input_report(&mut buffer) => {
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
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            reporter,
            listener,
        })
    }

    pub async fn send_report(&self, report: [u8; 32]) -> Result<()> {
        let mut report_with_id = [0u8; 33];
        report_with_id[1..33].copy_from_slice(&report);

        self.writer
            .lock()
            .await
            .write_output_report(&report_with_id)
            .await
            .map_err(|err| anyhow::Error::from(err))?;

        Ok(())
    }

    pub async fn request_report(&self, report: [u8; 32], ref_bytes: usize) -> Result<[u8; 32]> {
        let prefix = report[..ref_bytes].to_vec();
        let state = Arc::new(std::sync::Mutex::new(ReportFutureInner {
            data: None,
            waker: None,
        }));

        let (ack_tx, ack_rx) = oneshot::channel();
        self.reporter.send((prefix, state.clone(), ack_tx)).await?;
        ack_rx.await?;

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
