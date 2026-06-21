use crate::RepairAction;

pub fn action_confidence(action: RepairAction) -> f32 {
    match action {
        RepairAction::AllocateNewPort => 0.99,
        RepairAction::SwitchPackageManager => 0.95,
        RepairAction::InjectEnvironmentDefaults => 0.93,
        RepairAction::InstallDependency => 0.92,
        RepairAction::ChangeRuntimeVersion => 0.90,
        RepairAction::RebuildArtifacts => 0.89,
        RepairAction::RestartDependency => 0.87,
        RepairAction::RegenerateLockfile => 0.70,
    }
}

pub fn plan_confidence(actions: &[RepairAction], fallback: f32) -> f32 {
    if actions.is_empty() {
        return fallback;
    }
    actions
        .iter()
        .map(|action| action_confidence(*action))
        .product::<f32>()
        .min(0.99)
}
