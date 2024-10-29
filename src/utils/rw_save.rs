use std::{ops::{Deref, DerefMut}, sync::Arc};

use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{OwnedRwLockWriteGuard, RwLock, RwLockReadGuard};

pub struct RwSave<T> {
    file_path: String,
    rw_lock: Arc<RwLock<T>>,
}

pub struct RwSaveWriteGuard<'a, T: Serialize + Send + Sync + 'static> {
    path: &'a str,
    dirty: bool,
    guard: Option<OwnedRwLockWriteGuard<T>>,
}

impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> RwSave<T> {
    pub async fn new<F: FnOnce() -> T>(file_path: String, item_builder: F) -> Self {
        let item = 'load: {
            let data = match tokio::fs::read(&file_path).await {
                Ok(data) => data,
                Err(error) => {
                    tracing::error!(?error, "failed to load data from: {}", file_path);
                    break 'load None;
                }
            };

            match serde_json::from_slice(&data) {
                Ok(v) => Some(v),
                Err(error) => {
                    tracing::error!(?error, "failed to parse data");
                    None
                }
            }
        };

        RwSave {
            file_path,
            rw_lock: Arc::new(RwLock::new(item.unwrap_or_else(item_builder)))
        }
    }

    pub async fn write(&self) -> RwSaveWriteGuard<T> {
        let guard = self.rw_lock.clone().write_owned().await;

        RwSaveWriteGuard {
            path: &self.file_path,
            dirty: false,
            guard: Some(guard),
        }
    }

    pub async fn read(&self) -> RwLockReadGuard<T> {
        self.rw_lock.read().await
    }
}

impl<'a, T: Serialize + Send + Sync + 'static> Deref for RwSaveWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap().deref()
    }
}

impl<'a, T: Serialize + Send + Sync + 'static> DerefMut for RwSaveWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty = true;
        self.guard.as_mut().unwrap().deref_mut()
    }
}

impl<'a, T: Serialize + Send + Sync + 'static> Drop for RwSaveWriteGuard<'a, T> {
    fn drop(&mut self) {
        if !self.dirty {
            return;
        }

        let guard = self.guard.take().unwrap();
        let file_path = self.path.to_string();

        tokio::spawn(async move {
            let guard = guard.downgrade();
            let value = serde_json::to_string(&*guard).unwrap();
            
            match tokio::fs::write(&file_path, &value).await {
                Ok(_) => tracing::info!("Wrote dirty data for {} to {}", std::any::type_name::<T>(), file_path),
                Err(error) => tracing::error!(?error, "failed to write data to {}", file_path),
            }

            drop(guard);
        });
    }
}

