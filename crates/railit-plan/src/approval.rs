//! Freigabe eines Plan-Images im Vier-Augen-Prinzip (REQ-PLN-022).
//!
//! Die Signatur im Image (REQ-PLN-004) sagt, *wer* den Plan erzeugt hat. Eine **Freigabe** sagt,
//! dass ein Prüfer den Plan mit genau dieser Prüfsumme abgenommen hat. Sie liegt außerhalb des
//! Images (das Image ändert sich durch eine Freigabe nicht) und besteht aus dem öffentlichen
//! Schlüssel des Prüfers und dessen Ed25519-Signatur über
//! [`APPROVAL_CONTEXT`] ‖ Prüfsumme des Plans.
//!
//! Ein Plan gilt als freigegeben, wenn mindestens `required` **verschiedene** vertrauenswürdige
//! Prüfer ihn freigegeben haben und keiner davon der Ersteller ist (Schlüssel der Signatur im
//! Image). Wer einen Plan signiert, kann ihn nicht selbst freigeben.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::format::DIGEST_LEN;
use crate::view::PlanView;

/// Domänentrennung: Eine Freigabe ist nie zugleich eine gültige Plan- oder Siegelsignatur.
pub const APPROVAL_CONTEXT: &[u8; 24] = b"railit-plan-freigabe-v1\0";

/// Länge der signierten Nachricht.
const MESSAGE_LEN: usize = 24 + DIGEST_LEN;

/// Freigabe eines Prüfers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    /// Öffentlicher Schlüssel des Prüfers.
    pub approver: VerifyingKey,
    /// Signatur über [`APPROVAL_CONTEXT`] ‖ Prüfsumme.
    pub signature: Signature,
}

/// Grund, aus dem ein Plan nicht als freigegeben gilt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApprovalError {
    /// Zu wenige gültige Freigaben verschiedener vertrauenswürdiger Prüfer.
    TooFew {
        /// Gültige Freigaben.
        valid: usize,
        /// Verlangte Freigaben.
        required: usize,
    },
    /// Eine Freigabe passt nicht zum Plan (falscher Plan oder verfälscht).
    Invalid(usize),
    /// Eine Freigabe stammt von keinem vertrauenswürdigen Prüfer.
    Untrusted(usize),
    /// Der Ersteller hat seinen eigenen Plan freigegeben.
    SelfApproval(usize),
}

impl core::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooFew { valid, required } => write!(
                f,
                "Plan nicht freigegeben: {valid} von {required} verlangten Freigaben verschiedener Prüfer"
            ),
            Self::Invalid(i) => write!(
                f,
                "Freigabe {} passt nicht zu diesem Plan",
                i.saturating_add(1)
            ),
            Self::Untrusted(i) => write!(
                f,
                "Freigabe {} stammt von keinem vertrauenswürdigen Prüfer",
                i.saturating_add(1)
            ),
            Self::SelfApproval(i) => write!(
                f,
                "Freigabe {} stammt vom Ersteller des Plans (Vier-Augen-Prinzip)",
                i.saturating_add(1)
            ),
        }
    }
}

impl core::error::Error for ApprovalError {}

fn message(digest: &[u8; DIGEST_LEN]) -> [u8; MESSAGE_LEN] {
    let mut msg = [0u8; MESSAGE_LEN];
    let (head, tail) = msg.split_at_mut(APPROVAL_CONTEXT.len());
    head.copy_from_slice(APPROVAL_CONTEXT);
    tail.copy_from_slice(digest);
    msg
}

/// Gibt einen Plan mit der Prüfsumme `digest` frei.
#[must_use]
pub fn approve(digest: &[u8; DIGEST_LEN], key: &SigningKey) -> Approval {
    Approval {
        approver: key.verifying_key(),
        signature: key.sign(&message(digest)),
    }
}

impl Approval {
    /// Prüft die Freigabe gegen die Prüfsumme eines Plans.
    #[must_use]
    pub fn is_valid_for(&self, digest: &[u8; DIGEST_LEN]) -> bool {
        self.approver
            .verify_strict(&message(digest), &self.signature)
            .is_ok()
    }
}

/// Prüft die Freigaben eines Plans (REQ-PLN-022).
///
/// - Jede vorgelegte Freigabe muss zum Plan passen und von einem Schlüssel aus `trusted`
///   stammen; eine fremde oder verfälschte Freigabe weist den Plan ab, statt still übergangen
///   zu werden.
/// - `creator` ist der Schlüssel, gegen den die Signatur im Image geprüft wurde; dessen
///   Freigabe zählt nicht, sondern weist den Plan ab.
/// - Mehrfache Freigaben desselben Prüfers zählen einmal.
///
/// Gibt die Zahl verschiedener gültiger Prüfer zurück.
///
/// # Errors
/// [`ApprovalError`], wenn eine Freigabe ungültig ist oder zu wenige vorliegen.
pub fn check_approvals(
    plan: &PlanView<'_>,
    creator: Option<&VerifyingKey>,
    approvals: &[Approval],
    trusted: &[VerifyingKey],
    required: usize,
) -> Result<usize, ApprovalError> {
    let digest = plan.digest();
    let mut valid = 0usize;
    for (i, a) in approvals.iter().enumerate() {
        if creator == Some(&a.approver) {
            return Err(ApprovalError::SelfApproval(i));
        }
        if !trusted.contains(&a.approver) {
            return Err(ApprovalError::Untrusted(i));
        }
        if !a.is_valid_for(&digest) {
            return Err(ApprovalError::Invalid(i));
        }
        let earlier = approvals.get(..i).unwrap_or_default();
        if !earlier.iter().any(|e| e.approver == a.approver) {
            valid = valid.saturating_add(1);
        }
    }
    if valid < required {
        return Err(ApprovalError::TooFew { valid, required });
    }
    Ok(valid)
}
