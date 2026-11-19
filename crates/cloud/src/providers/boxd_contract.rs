use super::{
    ExecResult, ListedMachine, Machine, MachineState, MachineUsage, ProviderError, boxd::Boxd,
    contract::*,
};
use async_trait::async_trait;
use zuse_protocol::cloud_computers::ProviderId;

#[async_trait]
impl Provider for Boxd {
    fn id(&self) -> ProviderId {
        ProviderId::Boxd
    }
    async fn inspect(&self, id: &str) -> Result<Option<Machine>, ProviderError> {
        Boxd::inspect(self, id).await
    }
    async fn verify_initial_source(
        &self,
        machine: &str,
        artifact: &str,
        name: &str,
    ) -> Result<(), ProviderError> {
        self.verify_initial_snapshot_source(machine, artifact, name)
            .await
    }
    async fn find_allocation(&self, name: &str) -> Result<Option<Machine>, ProviderError> {
        Boxd::inspect(self, name).await
    }
    async fn allocate(&self, request: &Allocation<'_>) -> Result<Machine, ProviderError> {
        request.validate()?;
        self.verify_artifact(request.artifact, request.artifact_binding)
            .await?;
        // Plans sell `default` (4 vCPU / 16 GB) and `large` (8 vCPU / 32 GB).
        let (vcpu, memory_mib) = zuse_protocol::cloud_billing::plans::size_resources(request.size)
            .ok_or(ProviderError::Rejected)?;
        let machine = self
            .create_from_snapshot(request.artifact, request.name, true)
            .await?;
        // A machine inherits its snapshot's size. Resize only when it differs,
        // before the runtime starts; snapshots built at the plan size skip this.
        let memory = u64::from(memory_mib) * 1_048_576;
        let inherited = Boxd::inspect(self, &machine.id).await?;
        let current = inherited.as_ref().map(|m| (m.vcpu, m.memory_bytes));
        if current != Some((Some(vcpu), Some(memory))) {
            self.resize(&machine.id, vcpu, memory).await?;
        }
        Ok(machine)
    }
    async fn sleep(&self, id: &str) -> Result<ProviderOperation, ProviderError> {
        self.hibernate(id).await?;
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Sleeping,
        })
    }
    async fn sleep_memory(&self, id: &str) -> Result<ProviderOperation, ProviderError> {
        self.suspend(id).await?;
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Sleeping,
        })
    }
    async fn memory_operation(
        &self,
        receipt: &ProviderOperation,
    ) -> Result<OperationState, ProviderError> {
        if receipt.operation_id.is_some() || receipt.desired != DesiredState::Sleeping {
            return Err(ProviderError::InvalidResponse);
        }
        let machine = Boxd::inspect(self, &receipt.resource_id).await?;
        Ok(match machine.as_ref().map(|m| &m.state) {
            Some(MachineState::Standby) => OperationState::Completed,
            Some(MachineState::Paused | MachineState::Stopped) | None => OperationState::Superseded,
            _ => OperationState::Pending,
        })
    }
    async fn resume(
        &self,
        id: &str,
        lifetime_seconds: u32,
    ) -> Result<ProviderOperation, ProviderError> {
        if !(60..=86400).contains(&lifetime_seconds) {
            return Err(ProviderError::Rejected);
        }
        let machine = Boxd::inspect(self, id)
            .await?
            .ok_or(ProviderError::NotFound)?;
        match machine.state {
            MachineState::Running | MachineState::Starting => {}
            _ => self.activate(id, &machine.state).await?,
        }
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Running,
        })
    }
    async fn delete(&self, id: &str) -> Result<ProviderOperation, ProviderError> {
        match self.destroy(id).await {
            Ok(()) | Err(ProviderError::NotFound) => {}
            Err(error) => return Err(error),
        }
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Deleted,
        })
    }
    async fn operation(
        &self,
        receipt: &ProviderOperation,
    ) -> Result<OperationState, ProviderError> {
        if receipt.operation_id.is_some() {
            return Err(ProviderError::InvalidResponse);
        }
        Ok(super::contract::observed(
            receipt,
            Boxd::inspect(self, &receipt.resource_id).await?.as_ref(),
        ))
    }
    async fn execute(&self, id: &str, command: &str) -> Result<ExecResult, ProviderError> {
        self.exec(id, command).await
    }
    async fn upload(&self, id: &str, path: &str, contents: &[u8]) -> Result<(), ProviderError> {
        Boxd::upload(self, id, path, contents).await
    }
    async fn list_machines(&self) -> Result<Option<Vec<ListedMachine>>, ProviderError> {
        Boxd::list_machines(self).await.map(Some)
    }
    async fn usage_report(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Option<Vec<MachineUsage>>, ProviderError> {
        let usage = self.organization_usage(start_ms, end_ms).await?;
        Ok(Some(
            usage
                .into_iter()
                .map(|usage| MachineUsage {
                    machine_id: usage.machine_id,
                    running_seconds: usage.running_seconds,
                    cost_micros: usage.cost_micros,
                })
                .collect(),
        ))
    }
}
