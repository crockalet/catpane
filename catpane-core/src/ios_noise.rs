pub(crate) const IOS_SYSTEM_SUBSYSTEM_PREFIXES: &[&str] = &[
    "com.apple.",
    "com.apple.CoreSimulator.",
    "com.apple.WebKit.",
];

pub(crate) const IOS_SYSTEM_PROCESSES: &[&str] = &[
    "SpringBoard",
    "backboardd",
    "assertiond",
    "runningboardd",
    "launchd",
    "logd",
    "installd",
    "cfprefsd",
    "networkd",
    "nsurlsessiond",
    "powerd",
    "securityd",
    "symptomsd",
    "trustd",
    "wifid",
    "Simulator",
    "SimulatorTrampoline",
];

pub(crate) fn is_ios_system_subsystem(subsystem: &str) -> bool {
    IOS_SYSTEM_SUBSYSTEM_PREFIXES
        .iter()
        .any(|prefix| subsystem.starts_with(prefix))
}

pub(crate) fn is_ios_system_process(process: &str) -> bool {
    IOS_SYSTEM_PROCESSES
        .iter()
        .any(|candidate| process.eq_ignore_ascii_case(candidate))
}
