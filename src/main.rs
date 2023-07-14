use actix_web::dev::Server;
use actix_web::{
    get, middleware::Logger, post, put, web, App, HttpResponse, HttpServer, Responder,
};
use chrono::prelude::*;
use clap::Parser;
use env_logger;
use log;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, Result, Row};
use serde::{Deserialize, Serialize};
use serde_json::value::Serializer;
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{Error, ErrorKind};
use std::io::{Read, Write};
use std::process::Command;
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;
#[get("/hello/{name}")]
async fn greet(name: web::Path<String>) -> impl Responder {
    log::info!(target: "greet_handler", "Greeting {}", name);
    format!("Hello {name}!")
}

// DO NOT REMOVE: used in automatic testing
#[post("/internal/exit")]
#[allow(unreachable_code)]
async fn exit() -> impl Responder {
    log::info!("Shutdown as requested");
    std::process::exit(0);
    format!("Exited")
}

#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    config: Option<String>,
    #[arg(short, long)]
    flush_data: bool,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
enum ProblemType {
    Standard,
    Strict,
    Spj,
    DynamicRanking,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ServerInfo {
    bind_address: Option<String>,
    bind_port: Option<i32>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Misc {
    pack: Option<Vec<Vec<i32>>>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Case {
    score: f64,
    input_file: String,
    answer_file: String,
    time_limit: i32,
    memory_limit: i32,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Problem {
    id: i32,
    name: String,
    #[serde(rename = "type")]
    ty: ProblemType,
    misc: Option<Misc>,
    cases: Vec<Case>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Language {
    name: String,
    file_name: String,
    command: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Configure {
    server: ServerInfo,
    problems: Vec<Problem>,
    languages: Vec<Language>,
}
fn get_configure(json_path: String) -> Configure {
    let mut f = std::fs::File::open(json_path.clone()).unwrap();
    let mut buffer = String::new();
    f.read_to_string(&mut buffer).unwrap();
    let parse = serde_json::from_str(&buffer).unwrap();
    parse
}
#[derive(Serialize, Deserialize, Debug)]
struct Submit {
    source_code: String,
    language: String,
    user_id: i32,
    contest_id: i32,
    problem_id: i32,
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename = "SCREAMING_SNAKE_CASE")]
enum ErrorReason {
    ErrInvalidArgument,
    ErrInvalidState,
    ErrNotFound,
    ErrRateLimit,
    ErrExternal,
    ErrInternal,
}
#[derive(Serialize, Deserialize, Debug)]
struct ErrorMessage {
    code: i32,
    reason: ErrorReason,
    message: String,
}
#[derive(Serialize, Deserialize, Debug)]
enum State {
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
#[derive(Serialize, Deserialize, Debug)]
enum EnumResult {
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
#[derive(Serialize, Deserialize, Debug)]
struct CaseResult {
    id: i32,
    result: EnumResult,
    time: i32,
    memory: i32,
    info: String,
}
#[derive(Serialize, Deserialize, Debug)]
struct Message {
    id: i32,
    created_time: String,
    updated_time: String,
    submission: Submit,
    state: State,
    result: EnumResult,
    score: f64,
    cases: Vec<CaseResult>,
}
#[derive(Serialize, Deserialize, Debug)]
struct JobQuery {
    user_id: Option<i32>,
    user_name: Option<String>,
    contest_id: Option<i32>,
    problem_id: Option<i32>,
    language: Option<String>,
    from: Option<String>,
    to: Option<String>,
    state: Option<State>,
    result: Option<EnumResult>,
}
fn match_result(out_file: String, ans_file: String, compare: ProblemType) -> EnumResult {
    let mut out_stream = File::open(out_file).unwrap();
    let mut ans_stream = File::open(ans_file).unwrap();
    let mut buffer_out = String::new();
    let mut buffer_ans = String::new();
    out_stream.read_to_string(&mut buffer_out);
    ans_stream.read_to_string(&mut buffer_ans);
    match compare {
        ProblemType::Strict => match buffer_out.cmp(&mut buffer_ans) {
            std::cmp::Ordering::Equal => {
                return EnumResult::Accepted;
            }
            _ => {
                return EnumResult::WrongAnswer;
            }
        },
        _ => {
            buffer_ans = buffer_ans.trim_end().to_string();
            buffer_out = buffer_out.trim_end().to_string();
            let mut vec_ans: Vec<_> = buffer_ans.split('\n').collect();
            let mut vec_out: Vec<_> = buffer_out.split('\n').collect();
            if vec_ans.len() != vec_out.len() {
                return EnumResult::WrongAnswer;
            }
            for i in 0..vec_ans.len() {
                match vec_ans[i].trim_end().cmp(&mut vec_out[i].trim_end()) {
                    std::cmp::Ordering::Equal => {
                        continue;
                    }
                    _ => {
                        return EnumResult::WrongAnswer;
                    }
                }
            }
            return EnumResult::Accepted;
        }
    }
}
#[post("/jobs")]
async fn post_jobs(
    body: web::Json<Submit>,
    config: web::Data<Configure>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    //check if in config
    let created_time = Utc::now();
    let mut is_language = false;
    let mut is_problem = false;
    let mut problem_index = 0;
    let mut language_index = 0;
    for i in &config.problems {
        if i.id == body.problem_id {
            is_problem = true;
            break;
        }
        problem_index += 1;
    }
    let mut save_file_name = String::new();
    for i in &config.languages {
        if i.name == body.language {
            is_language = true;
            save_file_name = i.file_name.clone();
            break;
        }
        language_index += 1;
    }
    if is_problem == false {
        return HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Problem {} not found.", body.problem_id),
        });
    } else if is_language == false {
        return HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Language {} not found.", body.language),
        });
    } //TODO:比赛的检查
      //check user id
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM users WHERE id=?1",
            params![body.user_id],
            |row| row.get(0),
        )
        .unwrap();
    if conn == 0 {
        return HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("User {} not found.", body.user_id),
        });
    }
    //create temporary directory
    let mut vec_cases = Vec::new();
    for i in 0..config.problems[problem_index].cases.len() {
        vec_cases.push(CaseResult {
            id: i as i32,
            result: EnumResult::Waiting,
            time: 0,
            memory: 0,
            info: String::from(""),
        });
    }
    //determine id
    let mut id = 0;
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row("SELECT count(*) FROM foo", [], |row| row.get(0))
        .unwrap();
    if conn != 0 {
        let pool = pool.clone();
        let conn: usize = pool
            .get()
            .unwrap()
            .query_row("SELECT max(id) FROM task", [], |row| row.get(0))
            .unwrap();
        id = (conn as i32) + 1;
    }
    let mut message = Message {
        id: id,
        created_time: created_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        updated_time: created_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        submission: Submit {
            source_code: body.source_code.clone(),
            language: body.language.clone(),
            user_id: body.user_id,
            contest_id: body.contest_id,
            problem_id: body.problem_id,
        },
        state: State::Queueing,
        result: EnumResult::Waiting,
        score: 0.0,
        cases: vec_cases,
    };
    //insert entry in task table
    pool = pool.clone();
    let mut conn = pool.get().unwrap().execute(
        "INSERT INTO task VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        (
            &message.id,
            &message.submission.user_id,
            &message.submission.contest_id,
            &message.submission.problem_id,
            &message.submission.language,
            &message.created_time,
            message.state.to_string(),
            message.result.to_string(),
            &message.updated_time,
            &message.submission.source_code,
            &message.score,
            &serde_json::to_string(&message.cases).unwrap(),
        ),
    );
    //create folder
    let task_id = id;
    std::fs::create_dir(format!("temp{}", task_id));
    let folder_name = format!("temp{}", task_id);
    let src_path = format!("temp{}/{}", task_id, save_file_name);
    let mut buffer = std::fs::File::create(src_path.clone()).unwrap();
    buffer.write(body.source_code.as_bytes()).unwrap();
    let mut args_vec = config.languages[language_index as usize].command.clone();
    for mut i in &mut args_vec {
        if i == "%OUTPUT%" {
            i = &mut format!("{}/test", folder_name);
        } else if i == "%INPUT%" {
            i = &mut src_path.clone();
        }
    }
    let first_arg = args_vec.remove(0);
    let compile_time_start = Utc::now();
    message.state = State::Running;
    message.result = EnumResult::Running;
    message.cases[0].result = EnumResult::Running;
    message.updated_time = compile_time_start
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    //update task TABLE
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
        (
            "Running".to_string(),
            "Running".to_string(),
            serde_json::to_string(&message.cases).unwrap(),
            message.updated_time.clone(),
            message.id,
        ),
    );
    let status = Command::new(first_arg).args(args_vec).status();
    let updated_time = Utc::now();
    //update status
    message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    if let Err(r) = status {
        message.state = State::Finished;
        message.result = EnumResult::CompilationError;
        message.cases[0].result = EnumResult::CompilationError;
        pool = pool.clone();
        let conn = pool.get().unwrap().execute(
            "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
            (
                "Finished".to_string(),
                "Compilation Error".to_string(),
                serde_json::to_string(&message.cases).unwrap(),
                message.updated_time.clone(),
                message.id,
            ),
        );
        return HttpResponse::Ok().json(message);
    }
    message.cases[0].result = EnumResult::CompilationSuccess;
    //update task table
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET cases=?1,updated_time=?2 WHERE id=?3",
        (
            serde_json::to_string(&message.cases).unwrap(),
            message.updated_time.clone(),
            message.id,
        ),
    );
    let execute_path = format!("{}/test", folder_name);
    let mut index = 1;
    let mut output_path = Vec::new();
    //start executing program
    for i in &config.problems[problem_index].cases {
        output_path.push(format!("{}/{}.out", folder_name, index));
        let out_path = format!("{}/{}.out", folder_name, index);
        let mut child = Command::new(&execute_path)
            .arg("<")
            .arg(&i.input_file)
            .arg(">")
            .arg(&format!("{}/{}.out", folder_name, index))
            .spawn()
            .unwrap();
        let execute_start = Instant::now();
        let duration = Duration::from_micros(i.time_limit as u64);
        match child.wait_timeout(duration).unwrap() {
            Some(child_status) => {
                let execute_end = Instant::now();
                let t = execute_end.duration_since(execute_start);
                message.cases[index].time = t.as_micros() as i32;
                if !child_status.success() {
                    message.cases[index].result = EnumResult::RuntimeError;
                } else {
                    message.cases[index].result = match_result(
                        out_path,
                        config.problems[problem_index].cases[index]
                            .answer_file
                            .clone(),
                        config.problems[problem_index].ty.clone(),
                    );
                    if let EnumResult::Accepted = message.cases[index].result {
                        message.score += config.problems[problem_index].cases[index].score;
                    }
                }
            }
            None => {
                message.cases[index].time = i.time_limit;
                message.cases[index].result = EnumResult::TimeLimitExceeded;
                child.kill().unwrap();
                child.wait().unwrap();
            }
        }
        index += 1;
        //update task TABLE
        let updated_time = Utc::now();
        message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        pool = pool.clone();
        let conn = pool.get().unwrap().execute(
            "UPDATE task SET cases=?1,updated_time=?2,score=?3 WHERE id=?4",
            (
                serde_json::to_string(&message.cases).unwrap(),
                message.updated_time.clone(),
                message.score,
                message.id,
            ),
        );
    }
    //update final result
    message.state = State::Finished;
    message.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut is_accepted = true;
    let mut case_index = 0;
    for i in &message.cases {
        match i.result {
            EnumResult::Accepted => {
                continue;
            }
            EnumResult::RuntimeError => {
                message.result = EnumResult::RuntimeError;
                is_accepted = false;
                break;
            }
            EnumResult::WrongAnswer => {
                message.result = EnumResult::WrongAnswer;
                is_accepted = false;
                break;
            }
            _ => {}
        }
        case_index += 1;
    }
    if is_accepted {
        message.result = EnumResult::Accepted;
    }
    //update final result in task table
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET state=?1,result=?2,updated_time=?3,score=?4 WHERE id=?5",
        (
            "Finished".to_string(),
            serde_json::to_string(&message.result).unwrap(),
            message.updated_time.clone(),
            message.score,
            message.id,
        ),
    );
    std::fs::remove_dir_all(folder_name).unwrap();

    return HttpResponse::Ok().json(message);
}
#[get("/jobs")]
async fn get_jobs(
    info: web::Query<JobQuery>,
    config: web::Data<Configure>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    let mut query_str = String::new();
    if let Some(s) = info.user_id {
        query_str.push_str(&format!("user_id={}", s));
    }
    if let Some(s) = &info.user_name {
        pool = pool.clone();
        let conn: usize = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM users WHERE id=?1",
                params![s.clone()],
                |row| row.get(0),
            )
            .unwrap();
        if conn == 0 {
            let v: Vec<Message> = Vec::new();
            return HttpResponse::Ok().json(v);
        }
        let conn: usize = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE name=?1",
                params![s.clone()],
                |row| row.get(0),
            )
            .unwrap();
        if let Some(id) = info.user_id {
            if id as usize != conn {
                let v: Vec<Message> = Vec::new();
                return HttpResponse::Ok().json(v);
            }
        } else {
            if query_str.len() > 1 {
                query_str.push_str(" AND ");
            }
            query_str.push_str(&format!("user_id={}", conn as i32));
        }
    }
    if let Some(s) = info.contest_id {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("contest_id={}", s));
    }
    if let Some(s) = info.problem_id {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("problem_id={}", s));
    }
    if let Some(s) = &info.language {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("language={}", s));
    }
    if let Some(s) = &info.state {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("state={}", s.to_string()));
    }
    if let Some(s) = &info.result {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("result={}", s.to_string()));
    }
    if query_str.len() > 1 {
        let mut query_str = " WHERE ".to_string().push_str(&query_str);
    }
    pool = pool.clone();
    let mut conn = pool.get().unwrap();
    let mut t = conn
        .prepare(&format!("SELECT * FROM task{}", query_str))
        .unwrap();
    let tasks_iter = t
        .query_map([], |row| {
            Ok(Message {
                id: row.get(0)?,
                created_time: row.get(5)?,
                updated_time: row.get(8)?,
                submission: Submit {
                    source_code: row.get(9)?,
                    language: row.get(4)?,
                    user_id: row.get(1)?,
                    contest_id: row.get(2)?,
                    problem_id: row.get(3)?,
                },
                state: State::state_from_string(row.get(6).unwrap()),
                result: EnumResult::enumresult_from_string(row.get(7).unwrap()),
                score: row.get(10)?,
                cases: {
                    let mut t: String = row.get(11)?;
                    serde_json::from_str(&t).unwrap()
                },
            })
        })
        .unwrap();
    let mut vec_select = Vec::new();
    for i in tasks_iter {
        vec_select.push(i.unwrap());
    }
    //select from and to
    if let Some(r) = &info.from {
        let t = Utc.datetime_from_str(&r, "%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
        for mut i in 0..vec_select.len() as i32 {
            let ti = Utc
                .datetime_from_str(
                    &vec_select[i as usize].created_time,
                    "%Y-%m-%dT%H:%M:%S%.3fZ",
                )
                .unwrap();
            let duration = ti.signed_duration_since(t);
            if let core::cmp::Ordering::Less = duration.cmp(&chrono::Duration::zero()) {
                vec_select.remove(i as usize);
                i -= 1;
            }
        }
    }
    if let Some(r) = &info.to {
        let t = Utc.datetime_from_str(&r, "%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
        for mut i in 0..vec_select.len() as i32 {
            let ti = Utc
                .datetime_from_str(
                    &vec_select[i as usize].created_time,
                    "%Y-%m-%dT%H:%M:%S%.3fZ",
                )
                .unwrap();
            let duration = ti.signed_duration_since(t);
            if let core::cmp::Ordering::Greater = duration.cmp(&chrono::Duration::zero()) {
                vec_select.remove(i as usize);
                i -= 1;
            }
        }
    }
    //vec_select sort
    vec_select.sort_by(|a, b| {
        let ta = Utc
            .datetime_from_str(&a.created_time, "%Y-%m-%dT%H:%M:%S%.3fZ")
            .unwrap();
        let tb = Utc
            .datetime_from_str(&b.created_time, "%Y-%m-%dT%H:%M:%S%.3fZ")
            .unwrap();
        let duration = ta.signed_duration_since(tb);
        duration.cmp(&chrono::Duration::zero())
    });
    return HttpResponse::Ok().json(vec_select);
}
#[get("/jobs/{id}")]
async fn gets_job_id(
    id: web::Path<i32>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    pool = pool.clone();
    let mut conn = pool.get().unwrap();
    let mut t = conn
        .prepare(&format!("SELECT * FROM task WHERE id={}", id))
        .unwrap();
    let tasks_iter = t
        .query_map([], |row| {
            Ok(Message {
                id: row.get(0)?,
                created_time: row.get(5)?,
                updated_time: row.get(8)?,
                submission: Submit {
                    source_code: row.get(9)?,
                    language: row.get(4)?,
                    user_id: row.get(1)?,
                    contest_id: row.get(2)?,
                    problem_id: row.get(3)?,
                },
                state: State::state_from_string(row.get(6).unwrap()),
                result: EnumResult::enumresult_from_string(row.get(7).unwrap()),
                score: row.get(10)?,
                cases: {
                    let mut t: String = row.get(11)?;
                    serde_json::from_str(&t).unwrap()
                },
            })
        })
        .unwrap();
    let mut vec_select = Vec::new();
    for i in tasks_iter {
        vec_select.push(i.unwrap());
    }
    if vec_select.len() == 0 {
        return HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Job {} not found.", id),
        });
    }
    return HttpResponse::Ok().json(vec_select.pop().unwrap());
}
#[put("/jobs/{id}")]
async fn put_jobs(
    id: web::Path<i32>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
    config: web::Data<Configure>,
) -> impl Responder {
    pool = pool.clone();
    let mut conn = pool.get().unwrap();
    let mut t = conn
        .prepare(&format!("SELECT * FROM task WHERE id={}", id))
        .unwrap();
    let tasks_iter = t
        .query_map([], |row| {
            Ok(Message {
                id: row.get(0)?,
                created_time: row.get(5)?,
                updated_time: row.get(8)?,
                submission: Submit {
                    source_code: row.get(9)?,
                    language: row.get(4)?,
                    user_id: row.get(1)?,
                    contest_id: row.get(2)?,
                    problem_id: row.get(3)?,
                },
                state: State::state_from_string(row.get(6).unwrap()),
                result: EnumResult::enumresult_from_string(row.get(7).unwrap()),
                score: row.get(10)?,
                cases: {
                    let mut t: String = row.get(11)?;
                    serde_json::from_str(&t).unwrap()
                },
            })
        })
        .unwrap();
    let mut task_vec = Vec::new();
    for i in tasks_iter {
        task_vec.push(i.unwrap());
    }
    if task_vec.len() == 0 {
        return HttpResponse::NotFound().json(ErrorMessage {
            reason: ErrorReason::ErrNotFound,
            code: 3,
            message: format!("Job {} not found.", id),
        });
    }
    let mut message = task_vec.pop().unwrap();
    if let State::Finished = message.state {
    } else {
        return HttpResponse::BadRequest().json(ErrorMessage {
            reason: ErrorReason::ErrInvalidState,
            code: 2,
            message: format!("Job {} not finished.", id),
        });
    } //retest
    let mut language_index = 0;
    let mut save_file_name = String::new();
    for i in &config.languages {
        if i.name == message.submission.language {
            save_file_name = i.file_name.clone();
            break;
        }
        language_index += 1;
    }
    let task_id = id;
    std::fs::create_dir(format!("temp{}", task_id));
    let folder_name = format!("temp{}", task_id);
    let src_path = format!("temp{}/{}", task_id, save_file_name);
    let mut buffer = std::fs::File::create(src_path.clone()).unwrap();
    buffer
        .write(message.submission.source_code.as_bytes())
        .unwrap();
    let mut args_vec = config.languages[language_index as usize].command.clone();
    for mut i in &mut args_vec {
        if i == "%OUTPUT%" {
            i = &mut format!("{}/test", folder_name);
        } else if i == "%INPUT%" {
            i = &mut src_path.clone();
        }
    }
    let first_arg = args_vec.remove(0);
    let compile_time_start = Utc::now();
    message.state = State::Running;
    message.result = EnumResult::Running;
    message.cases[0].result = EnumResult::Running;
    message.updated_time = compile_time_start
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    //update task TABLE
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
        (
            "Running".to_string(),
            "Running".to_string(),
            serde_json::to_string(&message.cases).unwrap(),
            message.updated_time.clone(),
            message.id,
        ),
    );
    let status = Command::new(first_arg).args(args_vec).status();
    let updated_time = Utc::now();
    message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    if let Err(r) = status {
        message.state = State::Finished;
        message.result = EnumResult::CompilationError;
        message.cases[0].result = EnumResult::CompilationError;
        pool = pool.clone();
        let conn = pool.get().unwrap().execute(
            "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
            (
                "Finished".to_string(),
                "Compilation Error".to_string(),
                serde_json::to_string(&message.cases).unwrap(),
                message.updated_time.clone(),
                message.id,
            ),
        );
        return HttpResponse::Ok().json(message);
    }
    message.cases[0].result = EnumResult::CompilationSuccess;
    //upate task table
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET cases=?1,updated_time=?2 WHERE id=?3",
        (
            serde_json::to_string(&message.cases).unwrap(),
            message.updated_time.clone(),
            message.id,
        ),
    );
    let execute_path = format!("{}/test", folder_name);
    let mut index = 1;
    let mut output_path = Vec::new();
    for i in &config.problems[message.submission.problem_id as usize].cases {
        let out_path = format!("{}/{}.out", folder_name, index);
        output_path.push(out_path.clone());
        let mut child = Command::new(&execute_path)
            .arg("<")
            .arg(&i.input_file)
            .arg(">")
            .arg(&out_path)
            .spawn()
            .unwrap();
        let execute_start = Instant::now();
        let duration = Duration::from_micros(i.time_limit as u64);
        match child.wait_timeout(duration).unwrap() {
            Some(child_status) => {
                let execute_end = Instant::now();
                let t = execute_end.duration_since(execute_start);
                message.cases[index].time = t.as_micros() as i32;
                if !child_status.success() {
                    message.cases[index].result = EnumResult::RuntimeError;
                } else {
                    message.cases[index].result = match_result(
                        out_path,
                        config.problems[message.submission.problem_id as usize].cases[index]
                            .answer_file
                            .clone(),
                        config.problems[message.submission.problem_id as usize]
                            .ty
                            .clone(),
                    );
                    if let EnumResult::Accepted = message.cases[index].result {
                        message.score += config.problems[message.submission.problem_id as usize]
                            .cases[index]
                            .score;
                    }
                }
            }
            None => {
                message.cases[index].time = i.time_limit;
                message.cases[index].result = EnumResult::TimeLimitExceeded;
                child.kill().unwrap();
                child.wait().unwrap();
            }
        }
        index += 1;
        //update task TABLE
        let updated_time = Utc::now();
        message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        pool = pool.clone();
        let conn = pool.get().unwrap().execute(
            "UPDATE task SET cases=?1,updated_time=?2,score=?3 WHERE id=?4",
            (
                serde_json::to_string(&message.cases).unwrap(),
                message.updated_time.clone(),
                message.score,
                message.id,
            ),
        );
    }
    //update final result
    message.state = State::Finished;
    message.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut is_accepted = true;
    let mut case_index = 0;
    for i in &message.cases {
        match i.result {
            EnumResult::Accepted => {
                continue;
            }
            EnumResult::RuntimeError => {
                message.result = EnumResult::RuntimeError;
                is_accepted = false;
                break;
            }
            EnumResult::WrongAnswer => {
                message.result = EnumResult::WrongAnswer;
                is_accepted = false;
                break;
            }
            _ => {}
        }
        case_index += 1;
    }
    if is_accepted {
        message.result = EnumResult::Accepted;
    }
    //update final result in task table
    pool = pool.clone();
    let conn = pool.get().unwrap().execute(
        "UPDATE task SET state=?1,result=?2,updated_time=?3,score=?4 WHERE id=?5",
        (
            "Finished".to_string(),
            serde_json::to_string(&message.result).unwrap(),
            message.updated_time.clone(),
            message.score,
            message.id,
        ),
    );
    std::fs::remove_dir_all(folder_name).unwrap();
    return HttpResponse::Ok().json({});
}
#[derive(Serialize, Deserialize, Debug,Clone)]
struct User {
    id: i32,
    name: String,
}
#[derive(Serialize, Deserialize, Debug)]
struct InputUser {
    id: Option<i32>,
    name: String,
}
#[post("/users")]
async fn post_users(
    user: web::Path<InputUser>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM users WHERE name=?1",
            params![user.name.clone()],
            |row| row.get(0),
        )
        .unwrap();
    if conn != 0 {
        return HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: format!("User name '{}' already exists.", user.name.clone()),
        });
    }
    let mut user_id = 0;
    match user.id {
        Some(i) => {
            user_id = i;
            //check if in table
            pool = pool.clone();
            let conn: usize = pool
                .get()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM users WHERE id=?1",
                    params![i],
                    |row| row.get(0),
                )
                .unwrap();
            if conn == 0 {
                return HttpResponse::NotFound().json(ErrorMessage {
                    code: 3,
                    reason: ErrorReason::ErrNotFound,
                    message: format!("User {} not found.", i),
                });
            }
            pool = pool.clone();
            let conn = pool
                .get()
                .unwrap()
                .execute(
                    "UPDATE users SET name=?1 WHERE id=?2",
                    (user.name.clone(), user.id),
                )
                .unwrap();
        }
        None => {
            pool = pool.clone();
            let conn: i32 = pool
                .get()
                .unwrap()
                .query_row("SELECT max(id) FROM users", [], |row| row.get(0))
                .unwrap();
            let user_id = conn + 1;
            pool = pool.clone();
            pool.get().unwrap().execute(
                "INSERT INTO users (id,name) VALUES (?1,?2)",
                (user_id, user.name.clone()),
            );
        }
    }
    return HttpResponse::Ok().json(User {
        id: user_id,
        name: user.name.clone(),
    });
}
#[get("/users")]
async fn get_users(mut pool: web::Data<Pool<SqliteConnectionManager>>) -> impl Responder {
    pool = pool.clone();
    let conn = pool.get().unwrap();
    let mut prep = conn.prepare("SELECT * FROM users").unwrap();
    let user_iter = prep
        .query_map([], |row| {
            Ok(User {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .unwrap();
    let mut user_vec = Vec::new();
    for i in user_iter {
        user_vec.push(i.unwrap());
    }
    user_vec.sort_by(|a, b| a.id.cmp(&b.id));
    HttpResponse::Ok().json(user_vec)
}
#[derive(Serialize, Deserialize, Debug)]
enum ScoringRule {
    Latest,
    Highest,
}
#[derive(Clone)]
struct ScoringRuleStandard {
    submit_time: Option<DateTime<Utc>>,
    score: f64,
}
#[derive(Serialize, Deserialize, Debug,Clone)]
#[serde(rename_all = "snake_case")]
enum TieBreaker {
    SubmissionTime,
    SubmissionCount,
    UserId,
}
#[derive(Serialize, Deserialize, Debug)]
struct RankRule {
    scoring_rule: Option<ScoringRule>,
    tie_breaker: Option<TieBreaker>,
}

struct RanklistEntry {
    user: User,
    rank: i32,
    final_score: f64,
    scores: Vec<ScoringRuleStandard>,
}
#[derive(Serialize)]
struct RanklistReturn {
    user: User,
    rank: i32,
    scores: Vec<f64>,
}
fn sort_by_standard(
    a: &RanklistEntry,
    b: &RanklistEntry,
    standard: Option<TieBreaker>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> Ordering {
    match standard {
        None => {
            if a.final_score > b.final_score {
                return Ordering::Less;
            }
            if a.final_score < b.final_score {
                return Ordering::Greater;
            } else {
                return Ordering::Equal;
            }
        }
        Some(r) => match r {
            TieBreaker::SubmissionCount => {
                let conna: usize = pool
                    .get()
                    .unwrap()
                    .query_row(
                        "SELECT count(*) FROM tasks WHERE user_id=?1",
                        params![a.user.id],
                        |row| row.get(0),
                    )
                    .unwrap();
                let connb: usize = pool
                    .get()
                    .unwrap()
                    .query_row(
                        "SELECT count(*) FROM tasks WHERE user_id=?1",
                        params![b.user.id],
                        |row| row.get(0),
                    )
                    .unwrap();
                if a.final_score > b.final_score {
                    return Ordering::Less;
                }
                if a.final_score < b.final_score {
                    return Ordering::Greater;
                } else {
                    return conna.cmp(&connb);
                }
            }TieBreaker::UserId=>{
                if a.final_score>b.final_score{
                    return Ordering::Less;
                }if a.final_score<b.final_score{
                    return Ordering::Greater;
                }else{
                    return a.user.id.cmp(&b.user.id);
                }
            }TieBreaker::SubmissionTime=>{
                if a.final_score>b.final_score{
                    return Ordering::Less;
                }if a.final_score<b.final_score{
                    return Ordering::Greater;
                }else{
                    let mut submit_time_a:DateTime<Utc>=Utc.datetime_from_str("4022-08-27T02:05:29.000Z","%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
                    let mut submit_time_b:DateTime<Utc>=submit_time_a.clone();
                    for i in &a.scores{
                        match i.submit_time{
                            Some(t)=>{
                                if t<submit_time_a{
                                    submit_time_a=t.clone();
                                }
                            }None=>{}
                        }
                    }for i in &b.scores{
                        match i.submit_time{
                            Some(t)=>{
                                if t<submit_time_b{
                                    submit_time_b=t.clone();
                                }
                            }None=>{}
                        }
                    }if submit_time_a<submit_time_b{
                        return Ordering::Less;
                    }else if submit_time_a>submit_time_b{
                        return Ordering::Greater;
                    }else {return Ordering::Equal;}
                }
            }
        }
    }
}
#[get("/contests/{contestid}/ranklist")]
async fn get_contest_ranklist(
    contestid: web::Path<i32>,
    rank_rule: web::Query<RankRule>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
    config: web::Data<Configure>,
) -> impl Responder {
    let mut vec_ranklist: Vec<RanklistEntry> = Vec::new();
    if contestid == 0.into() {
        //create ranklist vector,get all the users
        pool = pool.clone();
        let c1 = pool.get().unwrap();
        let mut conn = c1.prepare("SELECT * FROM users").unwrap();
        let users_iter = conn
            .query_map([], |row| {
                Ok(User {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .unwrap();
        for i in users_iter {
            vec_ranklist.push(RanklistEntry {
                user: i.unwrap(),
                rank: 0,
                final_score: 0.0,
                scores: Vec::new(),
            });
        }
        //for every user,iterate all problems
        let mut vec_problem_id = Vec::new();
        for i in &config.problems {
            vec_problem_id.push(i.id);
        }
        vec_problem_id.sort();
        for user in &mut vec_ranklist {
            for prob_id in &vec_problem_id {
                let c1 = pool.get().unwrap();
                let mut conn = c1
                    .prepare(
                        "SELECT created_time,score FROM task WHERE user_id=?1 AND problem_id=?2",
                    )
                    .unwrap();
                let standard_iter = conn
                    .query_map([user.user.id, *prob_id], |row| {
                        let s: String = row.get(0)?;
                        Ok(ScoringRuleStandard {
                            submit_time: Some(
                                Utc.datetime_from_str(&s, "%Y-%m-%dT%H:%M:%S%.3fZ").unwrap(),
                            ),
                            score: row.get(1)?,
                        })
                    })
                    .unwrap();
                let mut vec_submit = Vec::new();
                for i in standard_iter {
                    vec_submit.push(i.unwrap());
                }
                if vec_submit.len() == 0 {
                    user.scores.push(ScoringRuleStandard {
                        submit_time: None,
                        score: (0.0),
                    });
                } else {
                    match rank_rule.scoring_rule {
                        Some(ScoringRule::Highest) => {
                            vec_submit.sort_by(|a, b| {
                                if a.score > b.score {
                                    std::cmp::Ordering::Less
                                } else if a.score == b.score {
                                    if let Some(atime) = a.submit_time {
                                        if let Some(btime) = b.submit_time {
                                            if atime < btime {
                                                Ordering::Less
                                            } else {
                                                Ordering::Greater
                                            }
                                        } else {
                                            Ordering::Less
                                        }
                                    } else {
                                        Ordering::Less
                                    }
                                } else {
                                    Ordering::Greater
                                }
                            });
                            user.scores.push(vec_submit[0].clone());
                        }
                        _ => {
                            vec_submit.sort_by(|a, b| {
                                if let Some(atime) = a.submit_time {
                                    if let Some(btime) = b.submit_time {
                                        if atime < btime {
                                            Ordering::Greater
                                        } else {
                                            Ordering::Less
                                        }
                                    } else {
                                        Ordering::Less
                                    }
                                } else {
                                    Ordering::Less
                                }
                            });
                            user.scores.push(vec_submit[0].clone());
                        }
                    }
                }
            } //update final score
            for i in &user.scores {
                user.final_score += i.score;
            }
        }
        //sort vec_ranklist
        vec_ranklist.sort_by(|a,b|{
            match sort_by_standard(a, b, rank_rule.tie_breaker.clone(), pool.clone()){
                Ordering::Greater=>{Ordering::Greater}
                Ordering::Less=>{Ordering::Less}
                Ordering::Equal=>{a.user.id.cmp(&b.user.id)}
            }
        });
        //generate return list
        let mut vec_return=Vec::new();
        for i in 0..vec_ranklist.len(){
            if i==0{
                vec_return.push(RanklistReturn{
                    user:vec_ranklist[i].user.clone(),
                    rank:1,
                    scores:{
                        let mut vec_scores=Vec::new();
                        for it in &vec_ranklist[i].scores{
                            vec_scores.push(it.score);
                        }vec_scores
                    }
                });
            }else{
                let mut rank=0;
                match sort_by_standard(&vec_ranklist[i-1], &vec_ranklist[i],rank_rule.tie_breaker.clone(), pool.clone()){
                    Ordering::Equal=>{
                        rank=vec_return[i-1].rank;
                    }_=>{
                        rank=vec_return[i-1].rank+1;
                    }
                }vec_return.push(RanklistReturn{
                    user:vec_ranklist[i].user.clone(),
                    rank:rank,
                    scores:{
                        let mut vec_scores=Vec::new();
                        for it in &vec_ranklist[i].scores{
                            vec_scores.push(it.score);
                        }vec_scores
                    }
                });
            }
        }return HttpResponse::Ok().json(vec_return);
    }
    HttpResponse::Ok().json({})
}
#[derive(Serialize,Deserialize)]
struct Contest{
    id:Option<i32>,
    name:String,
    from:String,
    to:String,
    problem_ids:Vec<i32>,
    user_ids:Vec<i32>,
    submission_limit:i32,
}
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    let cli = Cli::parse();
    let config: Configure;
    match cli.config {
        None => {
            return Err(Error::new(
                ErrorKind::Other,
                "your config file is not provided",
            ));
        }
        Some(r) => {
            config = get_configure(r);
        }
    }
    let mut current_task_id = 0;
    //init sql
    let server_address = match &config.server.bind_address {
        Some(r) => r.clone(),
        None => "127.0.0.1".to_string(),
    };
    let port_address = match config.server.bind_port {
        Some(r) => r,
        None => 12345,
    };
    let manager = SqliteConnectionManager::file("file.db");
    let mut pool = r2d2::Pool::new(manager).unwrap();
    pool.get()
        .unwrap()
        .execute(
            "
    CREATE TABLE IF NOT EXISTS task (
        id   INTEGER PRIMARY KEY,
        user_id INTEGER PRIMARY KEY,
        contest_id INTEGER PRIMARY KEY,
        problem_id INTEGER PRIMARY KEY,
        language TEXT NOT NULL,
        created_time TEXT NOT NULL,
        state TEXT NOT NULL,
        result TEXT NOT NULL,
        updated_time TEXT NOT NULL,
        source_code TEXT NOT NULL,
        score REAL,
        cases TEXT,
    )
    ",
            params![],
        )
        .unwrap();
    pool = pool.clone();
    pool.get()
        .unwrap()
        .execute(
            "
    CREATE TABLE IF NOT EXISTS users{
        id INTERGER PRIMARY KEY,
        name TEXT NOT NULL,
    }
    ",
            params![],
        )
        .unwrap();
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row("SELECT count(*) FROM users WHERE id=0", [], |row| {
            row.get(0)
        })
        .unwrap();
    if conn == 0 {
        pool = pool.clone();
        pool.get()
            .unwrap()
            .execute("INSERT INTO users (id, name) VALUES (?1, ?2)", (0, "root"));
    }
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(pool.clone()))
            .wrap(Logger::default())
            .route("/hello", web::get().to(|| async { "Hello World!" }))
            .service(greet)
            // DO NOT REMOVE: used in automatic testing
            .service(exit)
    })
    .bind(("127.0.0.1", 12345))?
    .run()
    .await
}
