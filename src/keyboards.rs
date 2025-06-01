use anyhow::Result;
use async_hid::{DeviceEvent, DeviceId, HidBackend};
use colored::Colorize;
use futures::StreamExt;
use indexmap::IndexMap;
use log::{debug, warn};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{
    Mutex as AsyncMutex, MutexGuard,
    broadcast::{self, Receiver, Sender},
};

use crate::{
    config::Config,
    consts::{QMK_USAGE_ID, QMK_USAGE_PAGE},
    keyboard::Keyboard,
};

#[derive(Clone)]
pub struct Keyboards {
    pub keyboards: Arc<AsyncMutex<IndexMap<DeviceId, AsyncMutex<Keyboard>>>>,
    configs: Arc<Mutex<HashMap<(u16, u16), Config>>>,
    sender: Sender<()>,
}

impl Keyboards {
    pub async fn from_configs(mut configs: HashMap<(u16, u16), Config>) -> Result<Self> {
        let mut keyboards = IndexMap::new();

        let backend = HidBackend::default();
        let mut stream = backend.enumerate().await?;

        while let Some(device) = stream.next().await {
            if device.usage_id == QMK_USAGE_ID && device.usage_page == QMK_USAGE_PAGE {
                if let Some(config) = configs.remove(&(device.vendor_id, device.product_id)) {
                    debug!("Keyboard {} connected!", config.name.bold());
                    match Keyboard::from_config(config, device).await {
                        Err(error) => warn!("Failed to initialize keyboard: {error}"),
                        Ok(keyboard) => {
                            keyboards
                                .insert(keyboard.device_id().clone(), AsyncMutex::new(keyboard));
                        }
                    }
                }
            }
        }

        Ok(Keyboards {
            configs: Arc::new(Mutex::new(configs)),
            keyboards: Arc::new(AsyncMutex::new(keyboards)),
            sender: broadcast::channel(32).0,
        })
    }

    pub fn watch(&self) -> Result<()> {
        let keyboards = self.keyboards.clone();
        let configs = self.configs.clone();
        let notifier = self.sender.clone();

        let backend = HidBackend::default();
        let mut watcher = backend.watch()?;

        tokio::spawn(async move {
            loop {
                if let Some(event) = watcher.next().await {
                    match event {
                        DeviceEvent::Connected(id) => {
                            let devices = backend.query_devices(&id).await.ok();
                            let device = devices.and_then(|mut x| x.next());
                            let config = device.as_ref().and_then(|device| {
                                let key = &(device.vendor_id, device.product_id);
                                configs.lock().unwrap().remove(key)
                            });

                            if let (Some(config), Some(device)) = (config, device) {
                                debug!("Keyboard {} connected!", config.name.bold());
                                match Keyboard::from_config(config, device).await {
                                    Err(error) => warn!("Failed to initialize keyboard: {error}"),
                                    Ok(keyboard) => {
                                        keyboards
                                            .lock()
                                            .await
                                            .insert(id, AsyncMutex::new(keyboard));

                                        _ = notifier.send(());
                                    }
                                }
                            }
                        }
                        DeviceEvent::Disconnected(id) => {
                            if let Some(device) = keyboards.lock().await.shift_remove(&id) {
                                let config = device.into_inner().into_config();
                                debug!("Keyboard {} disconnected!", config.name.bold());

                                configs
                                    .lock()
                                    .unwrap()
                                    .insert((config.vendor_id, config.product_id), config);

                                _ = notifier.send(());
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub fn subscribe(&self) -> Receiver<()> {
        self.sender.subscribe()
    }

    pub async fn items(&self) -> MutexGuard<'_, IndexMap<DeviceId, AsyncMutex<Keyboard>>> {
        self.keyboards.lock().await
    }
}
