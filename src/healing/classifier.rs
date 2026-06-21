use crate::{FailureClass, FailureClassifier, FailureSignal, RepositoryFingerprint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedFailure {
    pub class: FailureClass,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingClassifier {
    classifier: FailureClassifier,
}

impl HealingClassifier {
    pub fn classify(
        &self,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
    ) -> ClassifiedFailure {
        ClassifiedFailure {
            class: self.classifier.classify(failure, fingerprint),
            message: failure.message.clone(),
        }
    }
}
