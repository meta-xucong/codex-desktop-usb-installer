use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityState {
    Available,
    Unavailable,
    Unknown,
}

impl CapabilityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCheck {
    pub state: CapabilityState,
    pub detail: String,
}

impl CapabilityCheck {
    pub fn available(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Available,
            detail: detail.into(),
        }
    }

    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            detail: detail.into(),
        }
    }

    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unknown,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SideloadRecommendation {
    MsixPreferred,
    PortableFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinCapabilityReport {
    pub add_appx_package: CapabilityCheck,
    pub appx_service: CapabilityCheck,
    pub sideload_policy: CapabilityCheck,
    pub app_installer: CapabilityCheck,
    /// Can the WinRT `PackageManager` (the COM class `Add-AppxPackage` deploys
    /// through) actually be activated? Unlike the checks above, this exercises
    /// the deployment runtime itself — so it catches the "registered but broken"
    /// machines where MSIX install dies with `0x80040154 REGDB_E_CLASSNOTREG`
    /// ("没有注册类") even though the cmdlet, service and policy all look present.
    pub msix_deployment: CapabilityCheck,
    pub metered_network: CapabilityCheck,
    pub recommendation: SideloadRecommendation,
    pub notes: Vec<String>,
}

impl WinCapabilityReport {
    pub fn from_checks(
        add_appx_package: CapabilityCheck,
        appx_service: CapabilityCheck,
        sideload_policy: CapabilityCheck,
        app_installer: CapabilityCheck,
        msix_deployment: CapabilityCheck,
        metered_network: CapabilityCheck,
        mut notes: Vec<String>,
    ) -> Self {
        let msix_blocked = add_appx_package.state == CapabilityState::Unavailable
            || appx_service.state == CapabilityState::Unavailable
            || sideload_policy.state == CapabilityState::Unavailable
            || msix_deployment.state == CapabilityState::Unavailable;
        let recommendation = if msix_blocked {
            if msix_deployment.state == CapabilityState::Unavailable {
                // A recommendation is not permission to change installation
                // mode: fixed-MSIX USB bundles must still stop on this failure.
                notes.push(
                    "MSIX deployment runtime activation failed (for example, 0x80040154). A portable build is an option only when the selected installation mode permits it; fixed-MSIX USB installation cannot fall back."
                        .to_string(),
                );
            } else {
                notes.push(
                    "MSIX sideloading appears blocked; a portable fallback is only a recommendation for modes that explicitly permit it."
                        .to_string(),
                );
            }
            SideloadRecommendation::PortableFallback
        } else {
            if sideload_policy.state == CapabilityState::Unknown {
                notes.push(
                    "Sideload policy is not conclusively known; see the probe detail for an unconfigured value or a read error. This is not evidence that developer mode is required or that USB deployment is blocked. Installation mode controls whether any fallback is permitted."
                        .to_string(),
                );
            }
            SideloadRecommendation::MsixPreferred
        };
        let add_appx = add_appx_package.state.as_str();
        let appx_service_state = appx_service.state.as_str();
        let sideload_policy_state = sideload_policy.state.as_str();
        let msix_deployment_state = msix_deployment.state.as_str();
        let metered = metered_network.state.as_str();
        log::info!(
            "Windows capability probe result add_appx={add_appx} appx_service={appx_service_state} sideload_policy={sideload_policy_state} msix_deployment={msix_deployment_state} metered={metered}"
        );
        if recommendation == SideloadRecommendation::PortableFallback {
            let reason = notes
                .last()
                .map(String::as_str)
                .unwrap_or("MSIX sideload blocked");
            log::warn!("MSIX sideload blocked; recommending portable fallback reason={reason}");
        }

        Self {
            add_appx_package,
            appx_service,
            sideload_policy,
            app_installer,
            msix_deployment,
            metered_network,
            recommendation,
            notes,
        }
    }

    pub fn unknown_for_non_windows() -> Self {
        Self::from_checks(
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            vec!["Windows capability checks are only meaningful on Windows.".to_string()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_msix_when_no_blocking_signal_exists() {
        let report = WinCapabilityReport::from_checks(
            CapabilityCheck::available("present"),
            CapabilityCheck::available("running"),
            CapabilityCheck::unknown("policy absent"),
            CapabilityCheck::available("installed"),
            CapabilityCheck::available("PackageManager activates"),
            CapabilityCheck::unknown("not probed"),
            vec![],
        );
        assert_eq!(report.recommendation, SideloadRecommendation::MsixPreferred);
        assert!(report.notes.iter().any(|note| note.contains("read error")));
        assert!(!report
            .notes
            .iter()
            .any(|note| note.contains("will fall back")));
    }

    #[test]
    fn recommends_portable_when_policy_blocks_sideloading() {
        let report = WinCapabilityReport::from_checks(
            CapabilityCheck::available("present"),
            CapabilityCheck::available("running"),
            CapabilityCheck::unavailable("AllowAllTrustedApps=0"),
            CapabilityCheck::available("installed"),
            CapabilityCheck::available("PackageManager activates"),
            CapabilityCheck::unknown("not probed"),
            vec![],
        );
        assert_eq!(
            report.recommendation,
            SideloadRecommendation::PortableFallback
        );
    }

    #[test]
    fn recommends_portable_when_msix_deployment_is_broken() {
        // The 0x80040154 case: cmdlet, service and policy all look fine, but the
        // deployment runtime itself cannot activate, so sideloading cannot work.
        let report = WinCapabilityReport::from_checks(
            CapabilityCheck::available("present"),
            CapabilityCheck::available("running"),
            CapabilityCheck::available("AllowAllTrustedApps=1"),
            CapabilityCheck::available("installed"),
            CapabilityCheck::unavailable(
                "PackageManager activation failed: 没有注册类 (HRESULT=0x80040154)",
            ),
            CapabilityCheck::unknown("not probed"),
            vec![],
        );
        assert_eq!(
            report.recommendation,
            SideloadRecommendation::PortableFallback
        );
        assert!(report.notes.iter().any(|note| note.contains("0x80040154")));
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("fixed-MSIX USB installation cannot fall back")));
    }
}
