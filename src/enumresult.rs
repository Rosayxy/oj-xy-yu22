use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum EnumResult {
    Waiting,
    Running,
    Accepted,
    #[serde(rename = "Compilation Error")]
    CompilationError,
    #[serde(rename = "Compilation Success")]
    CompilationSuccess,
    #[serde(rename = "Wrong Answer")]
    WrongAnswer,
    #[serde(rename = "Runtime Error")]
    RuntimeError,
    #[serde(rename = "Time Limit Exceeded")]
    TimeLimitExceeded,
    #[serde(rename = "Memory Limit Exceeded")]
    MemoryLimitExceeded,
    #[serde(rename = "System Error")]
    SystemError,
    #[serde(rename = "SPJ Error")]
    SPJError,
    Skipped,
}
impl EnumResult {
    pub fn to_string(&self) -> String {
        match &self {
            Self::Accepted => "Accepted".to_string(),
            Self::CompilationError => "Compilation Error".to_string(),
            Self::CompilationSuccess => "Compilation Success".to_string(),
            Self::MemoryLimitExceeded => "Memory Limit Exceeded".to_string(),
            Self::Running => "Running".to_string(),
            Self::RuntimeError => "Runtime Error".to_string(),
            Self::SPJError => "SPJ Error".to_string(),
            Self::Skipped => "Skipped".to_string(),
            Self::SystemError => "System Error".to_string(),
            Self::TimeLimitExceeded => "Time Limit Exceeded".to_string(),
            Self::Waiting => "Waiting".to_string(),
            Self::WrongAnswer => "Wrong Answer".to_string(),
        }
    }
    pub fn enumresult_from_string(s: String) -> EnumResult {
        let mut t = "\"".to_string();
        t.push_str(&s);
        t.push_str("\"");
        let state: EnumResult = serde_json::from_str(&t).unwrap();
        return state;
    }
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum State {
    Queueing,
    Running,
    Finished,
    Canceled,
}
impl State {
    pub fn to_string(&self) -> String {
        match &self {
            Self::Queueing => "Queueing".to_string(),
            Self::Canceled => "Canceled".to_string(),
            Self::Finished => "Finished".to_string(),
            Self::Running => "Running".to_string(),
        }
    }
    pub fn state_from_string(s: String) -> State {
        let mut t = "\"".to_string();
        t.push_str(&s);
        t.push_str("\"");
        let state: State = serde_json::from_str(&t).unwrap();
        return state;
    }
}
