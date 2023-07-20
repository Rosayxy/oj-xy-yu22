use serde::{Deserialize, Serialize};
use std::io::Read;
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ProblemType {
    Standard,
    Strict,
    Spj,
    DynamicRanking,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerInfo {
    pub bind_address: Option<String>,
    pub bind_port: Option<u16>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Misc {
    pub packing: Option<Vec<Vec<i32>>>,
    pub dynamic_ranking_ratio: Option<f64>,
    pub special_judge: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Case {
    pub score: f64,
    pub input_file: String,
    pub answer_file: String,
    pub time_limit: i32,
    pub memory_limit: i32,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Problem {
    pub id: i32,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ProblemType,
    pub misc: Option<Misc>,
    pub cases: Vec<Case>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Language {
    pub name: String,
    pub file_name: String,
    pub command: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Configure {
    pub server: ServerInfo,
    pub problems: Vec<Problem>,
    pub languages: Vec<Language>,
}
pub fn get_configure(json_path: String) -> Configure {
    let mut f = std::fs::File::open(json_path.clone()).unwrap();
    let mut buffer = String::new();
    f.read_to_string(&mut buffer).unwrap();
    let parse: Configure = serde_json::from_str(&buffer).unwrap();
    parse
}
