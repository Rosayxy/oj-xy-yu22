use actix_web::{get, middleware::Logger, post, web, App, HttpResponse, HttpServer, Responder};
use chrono::prelude::*;
use clap::Parser;
use env_logger;
use log;
use serde::{Deserialize, Serialize};
use std::fs::File;
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
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum ProblemType {
    Standard,
    Strict,
    Spj,
    DynamicRanking,
}
#[derive(Serialize, Deserialize, Clone)]
struct Server {
    bind_address: Option<String>,
    bind_port: Option<i32>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Misc {
    pack: Option<Vec<Vec<i32>>>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Case {
    score: f64,
    input_file: String,
    answer_file: String,
    time_limit: i32,
    memory_limit: i32,
}
#[derive(Serialize, Deserialize, Clone)]
struct Problem {
    id: i32,
    name: String,
    #[serde(rename = "type")]
    ty: ProblemType,
    misc: Option<Misc>,
    cases: Vec<Case>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Language {
    name: String,
    file_name: String,
    command: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Configure {
    server: Server,
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
#[derive(Serialize, Deserialize)]
struct Submit {
    source_code: String,
    language: String,
    user_id: i32,
    contest_id: i32,
    problem_id: i32,
}
#[derive(Serialize, Deserialize)]
#[serde(rename = "SCREAMING_SNAKE_CASE")]
enum ErrorReason {
    ErrInvalidArgument,
    ErrInvalidState,
    ErrNotFound,
    ErrRateLimit,
    ErrExternal,
    ErrInternal,
}
#[derive(Serialize, Deserialize)]
struct ErrorMessage {
    code: i32,
    reason: ErrorReason,
    message: String,
}
#[derive(Serialize, Deserialize)]
enum State {
    Queueing,
    Running,
    Finished,
    Canceled,
}
#[derive(Serialize, Deserialize)]
enum Result {
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
#[derive(Serialize, Deserialize)]
struct CaseResult {
    id: i32,
    result: Result,
    time: i32,
    memory: i32,
    info: String,
}
#[derive(Serialize, Deserialize)]
struct Message {
    id: i32,
    created_time: String,
    updated_time: String,
    submission: Submit,
    state: State,
    result: Result,
    score: f64,
    cases: Vec<CaseResult>,
}
fn match_result(out_file: String, ans_file: String, compare: ProblemType) -> Result {
    let mut out_stream = File::open(out_file).unwrap();
    let mut ans_stream = File::open(ans_file).unwrap();
    let mut buffer_out = String::new();
    let mut buffer_ans = String::new();
    out_stream.read_to_string(&mut buffer_out);
    ans_stream.read_to_string(&mut buffer_ans);
    match compare {
        ProblemType::Strict => match buffer_out.cmp(&mut buffer_ans) {
            std::cmp::Ordering::Equal => {
                return Result::Accepted;
            }
            _ => {
                return Result::WrongAnswer;
            }
        },
        _ => {
            buffer_ans = buffer_ans.trim_end().to_string();
            buffer_out = buffer_out.trim_end().to_string();
            let mut vec_ans: Vec<_> = buffer_ans.split('\n').collect();
            let mut vec_out: Vec<_> = buffer_out.split('\n').collect();
            if vec_ans.len() != vec_out.len() {
                return Result::WrongAnswer;
            }
            for i in 0..vec_ans.len() {
                match vec_ans[i].trim_end().cmp(&mut vec_out[i].trim_end()) {
                    std::cmp::Ordering::Equal => {
                        continue;
                    }
                    _ => {
                        return Result::WrongAnswer;
                    }
                }
            }
            return Result::Accepted;
        }
    }
}
#[post("/jobs")]
async fn post_jobs(body: web::Json<Submit>, config: web::Data<Configure>) -> impl Responder {
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
    if (is_problem == false) {
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
    } //TODO:用户id和比赛的检查
      //create temporary directory
    let mut vec_cases = Vec::new();
    for i in 0..config.problems[problem_index].cases.len() {
        vec_cases.push(CaseResult {
            id: i as i32,
            result: Result::Waiting,
            time: 0,
            memory: 0,
            info: String::from(""),
        });
    }
    let mut message = Message {
        id: 0,
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
        result: Result::Waiting,
        score: 0.0,
        cases: vec_cases,
    };
    let task_id = 0;
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
    message.result = Result::Running;
    message.cases[0].result = Result::Running;
    let status = Command::new(first_arg).args(args_vec).status();
    if let Err(r) = status {
        let updated_time = Utc::now();
        message.state = State::Finished;
        message.result = Result::CompilationError;
        message.cases[0].result = Result::CompilationError;
        message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        return HttpResponse::Ok().json(message);
    }
    message.cases[0].result = Result::CompilationSuccess;
    let execute_path = format!("{}/test", folder_name);
    let mut index = 1;
    let mut output_path = Vec::new();
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
                    message.cases[index].result = Result::RuntimeError;
                } else {
                    message.cases[index].result = match_result(
                        out_path,
                        config.problems[problem_index].cases[index]
                            .answer_file
                            .clone(),
                        config.problems[problem_index].ty.clone(),
                    );
                }
            }
            None => {
                message.cases[index].time = i.time_limit;
                message.cases[index].result = Result::TimeLimitExceeded;
                child.kill().unwrap();
                child.wait().unwrap();
            }
        }
        index += 1;
    }
    //update final result
    message.state = State::Finished;
    message.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut is_accepted = true;
    let mut case_index = 0;
    for i in &message.cases {
        match i.result {
            Result::Accepted => {
                message.score += config.problems[problem_index].cases[case_index].score;
                continue;
            }
            Result::RuntimeError => {
                message.result = Result::RuntimeError;
                is_accepted = false;
                break;
            }
            Result::WrongAnswer => {
                message.result = Result::WrongAnswer;
                is_accepted = false;
                break;
            }
            _ => {}
        }
        case_index += 1;
    }
    if is_accepted {
        message.result = Result::Accepted;
    }
    return HttpResponse::Ok().json(message);
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
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
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
