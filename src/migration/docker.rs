//! Docker lifecycle actions are bound to full IDs and verified ownership.
use super::container::{LegacyContainer, Namespace, Network};
use anyhow::{Context, Result, ensure};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    query_parameters::{
        ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, StopContainerOptionsBuilder,
    },
};
use serde_json::Value;
use std::collections::HashMap;

pub struct Containers {
    docker: Docker,
    runtime: tokio::runtime::Runtime,
}

impl Containers {
    pub fn connect(socket: &str) -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_socket(socket, 45, API_DEFAULT_VERSION)?,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    async fn inspect_async(&self, id: &str) -> Result<Option<Value>> {
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "full Docker ID required"
        );
        match self.docker.inspect_container(id, None).await {
            Ok(value) => Ok(Some(serde_json::to_value(value)?)),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(_) => anyhow::bail!("Docker identity inspection failed"),
        }
    }

    pub fn inspect(&self, id: &str) -> Result<Option<Value>> {
        self.runtime.block_on(self.inspect_async(id))
    }

    async fn network_async(&self, id: &str) -> Result<Option<Value>> {
        match self.docker.inspect_network(id, None).await {
            Ok(value) => Ok(Some(serde_json::to_value(value)?)),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(_) => anyhow::bail!("Docker network inspection failed"),
        }
    }

    pub fn network(&self, namespace: Namespace) -> Result<Option<Value>> {
        self.runtime
            .block_on(self.network_async(&format!("{}-internal", namespace.name())))
    }

    pub fn remove_network(&self, expected: &Network, namespace: Namespace) -> Result<()> {
        self.runtime.block_on(async {
            if let Some(value) = self.network_async(&expected.id).await? {
                expected.verify(&value, namespace)?;
                ensure!(
                    value["Containers"]
                        .as_object()
                        .is_some_and(serde_json::Map::is_empty),
                    "network still has attached containers"
                );
                self.docker
                    .remove_network(&expected.id)
                    .await
                    .map_err(|_| anyhow::anyhow!("exact network removal failed"))?;
            }
            ensure!(
                self.network_async(&expected.id).await?.is_none(),
                "network cleanup incomplete"
            );
            Ok(())
        })
    }

    pub fn inventory(&self, namespace: Namespace) -> Result<Vec<Value>> {
        self.runtime.block_on(async {
            let options = ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&HashMap::from([(
                    "label",
                    vec![format!("{}.managed=true", namespace.prefix())],
                )]))
                .build();
            let rows = self
                .docker
                .list_containers(Some(options))
                .await
                .map_err(|_| anyhow::anyhow!("Docker inventory failed"))?;
            ensure!(
                rows.len() <= 64,
                "managed container inventory exceeds migration limit"
            );
            let mut result = Vec::new();
            for row in rows {
                result.push(
                    self.inspect_async(row.id.as_deref().context("container ID missing")?)
                        .await?
                        .context("container disappeared during preflight")?,
                );
            }
            Ok(result)
        })
    }

    async fn stop_async(&self, expected: &LegacyContainer, namespace: Namespace) -> Result<()> {
        let Some(value) = self.inspect_async(&expected.container_id).await? else {
            return Ok(());
        };
        expected.verify_as(&value, namespace)?;
        let running = value
            .pointer("/State/Running")
            .and_then(Value::as_bool)
            .context("container state missing")?;
        if running {
            self.docker
                .stop_container(
                    &expected.container_id,
                    Some(StopContainerOptionsBuilder::default().t(30).build()),
                )
                .await
                .map_err(|_| anyhow::anyhow!("exact container stop failed"))?;
        }
        if let Some(stopped) = self.inspect_async(&expected.container_id).await? {
            expected.verify_as(&stopped, namespace)?;
            ensure!(
                stopped.pointer("/State/Running").and_then(Value::as_bool) == Some(false),
                "container is still running"
            );
        }
        Ok(())
    }

    pub fn stop_exact(&self, expected: &LegacyContainer, namespace: Namespace) -> Result<()> {
        self.runtime.block_on(self.stop_async(expected, namespace))
    }

    pub fn remove_exact(&self, expected: &LegacyContainer, namespace: Namespace) -> Result<()> {
        self.runtime.block_on(async {
            self.stop_async(expected, namespace).await?;
            if let Some(value) = self.inspect_async(&expected.container_id).await? {
                expected.verify_as(&value, namespace)?;
                self.docker
                    .remove_container(
                        &expected.container_id,
                        Some(
                            RemoveContainerOptionsBuilder::default()
                                .force(false)
                                .v(false)
                                .build(),
                        ),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("exact container removal failed"))?;
            }
            ensure!(
                self.inspect_async(&expected.container_id).await?.is_none(),
                "exact container cleanup is incomplete"
            );
            Ok(())
        })
    }

    pub fn restore_legacy(&self, expected: &LegacyContainer) -> Result<()> {
        self.runtime.block_on(async {
            if let Some(value) = self.inspect_async(&expected.container_id).await? {
                expected.verify(&value)?;
                if value.pointer("/State/Running").and_then(Value::as_bool) == Some(false) {
                    self.docker
                        .start_container(&expected.container_id, None)
                        .await
                        .map_err(|_| anyhow::anyhow!("legacy container restart failed"))?;
                }
            }
            Ok(())
        })
    }
}
