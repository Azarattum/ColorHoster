use anyhow::Result;
use async_hid::{AsyncHidRead, AsyncHidWrite, Device, DeviceId, DeviceWriter};
use std::sync::Arc;
use tokio::sync::{
    Mutex as AsyncMutex,
    mpsc::{self, Sender},
    oneshot,
};
use tokio_util::sync::CancellationToken;

use crate::report::{FutureReport, FutureReportState, Report};

type ReportRequest<const N: usize> = (Vec<u8>, FutureReportState<N>, oneshot::Sender<()>);

pub struct KeyboardDevice<const N: usize> {
    writer: Arc<AsyncMutex<DeviceWriter>>,
    listener: CancellationToken,
    reporter: Sender<ReportRequest<N>>,
    pub id: DeviceId,
}

impl<const N: usize> KeyboardDevice<N> {
    pub fn create_report(&self) -> Report<N> {
        Report::<N>::new()
    }

    pub async fn from_device(device: Device) -> Result<Self> {
        let (mut reader, writer) = device.open().await?;

        let listener = CancellationToken::new();
        let signal = listener.clone();

        let (reporter, mut receiver) = mpsc::channel::<ReportRequest<N>>(32);

        tokio::spawn(async move {
            let mut requests: Vec<(Vec<u8>, FutureReportState<N>)> = Vec::new();

            loop {
                let mut buffer = [0u8; N];
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
            id: device.id.clone(),
            reporter,
            listener,
        })
    }

    pub async fn send_report(&self, report: Report<N>) -> Result<()> {
        self.writer
            .lock()
            .await
            .write_output_report(&report.into_inner())
            .await
            .map_err(|err| anyhow::Error::from(err))?;

        Ok(())
    }

    pub async fn request_report(&self, report: Report<N>, ref_bytes: usize) -> Result<[u8; N]> {
        let prefix = report[..ref_bytes].to_vec();
        let state = FutureReport::new_state();

        let (ack_tx, ack_rx) = oneshot::channel();
        self.reporter.send((prefix, state.clone(), ack_tx)).await?;
        ack_rx.await?;

        self.send_report(report).await?;
        Ok(FutureReport::from_state(state).await)
    }
}

impl<const N: usize> Drop for KeyboardDevice<N> {
    fn drop(&mut self) {
        self.listener.cancel();
    }
}
