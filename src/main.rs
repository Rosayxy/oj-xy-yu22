use actix_web::{get, middleware::Logger, post, web, App, HttpResponse, HttpServer, Responder};
use actix_web::dev::Server;
use chrono::prelude::*;
use clap::Parser;
use env_logger;
use log;
use serde::{Deserialize, Serialize};
use std::fs::{File, self};
use std::io::{Error, ErrorKind};
use std::io::{Read, Write};
use std::process::Command;
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;
use rusqlite::{params, Connection, Result};
use r2d2_sqlite::SqliteConnectionManager;
use r2d2::Pool;
use serde_json::value::Serializer;
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
#[derive(Serialize, Deserialize, Clone,Debug)]
#[serde(rename_all = "snake_case")]
enum ProblemType {
    Standard,
    Strict,
    Spj,
    DynamicRanking,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
struct ServerInfo {
    bind_address: Option<String>,
    bind_port: Option<i32>,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
struct Misc {
    pack: Option<Vec<Vec<i32>>>,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
struct Case {
    score: f64,
    input_file: String,
    answer_file: String,
    time_limit: i32,
    memory_limit: i32,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
struct Problem {
    id: i32,
    name: String,
    #[serde(rename = "type")]
    ty: ProblemType,
    misc: Option<Misc>,
    cases: Vec<Case>,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
struct Language {
    name: String,
    file_name: String,
    command: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone,Debug)]
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
#[derive(Serialize, Deserialize,Debug)]
struct Submit {
    source_code: String,
    language: String,
    user_id: i32,
    contest_id: i32,
    problem_id: i32,
}
#[derive(Serialize, Deserialize,Debug)]
#[serde(rename = "SCREAMING_SNAKE_CASE")]
enum ErrorReason {
    ErrInvalidArgument,
    ErrInvalidState,
    ErrNotFound,
    ErrRateLimit,
    ErrExternal,
    ErrInternal,
}
#[derive(Serialize, Deserialize,Debug)]
struct ErrorMessage {
    code: i32,
    reason: ErrorReason,
    message: String,
}
#[derive(Serialize, Deserialize,Debug)]
enum State {
    Queueing,
    Running,
    Finished,
    Canceled,
}
impl State{
    pub fn to_string(&self)->String{
        match &self{
            Self::Queueing=>{
                "Queueing".to_string()
            }Self::Canceled=>{
                "Canceled".to_string()
            }Self::Finished=>{
                "Finished".to_string()
            }Self::Running=>{
                "Running".to_string()
            }
        }
    }pub fn state_from_string(s:String)->State{
        let mut t="\"".to_string();
        t.push_str(&s);
        t.push_str("\"");
        let state:State=serde_json::from_str(&t).unwrap();
        return state;
    }
}
#[derive(Serialize, Deserialize,Debug)]
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
impl EnumResult{
    pub fn to_string(&self)->String{
        match &self{
            Self::Accepted=>{
                "Accepted".to_string()
            }Self::CompilationError=>{
                "Compilation Error".to_string()
            }Self::CompilationSuccess=>{
                "Compilation Success".to_string()
            }Self::MemoryLimitExceeded=>{
                "Memory Limit Exceeded".to_string()
            }Self::Running=>{
                "Running".to_string()
            }Self::RuntimeError=>{
                "Runtime Error".to_string()
            }Self::SPJError=>{
                "SPJ Error".to_string()
            }Self::Skipped=>{
                "Skipped".to_string()
            }Self::SystemError=>{
                "System Error".to_string()
            }Self::TimeLimitExceeded=>{
                "Time Limit Exceeded".to_string()
            }Self::Waiting=>{
                "Waiting".to_string()
            }Self::WrongAnswer=>{
                "Wrong Answer".to_string()
            }
        }
    }
    pub fn enumresult_from_string(s:String)->EnumResult{
        let mut t="\"".to_string();
        t.push_str(&s);
        t.push_str("\"");
        let state:EnumResult=serde_json::from_str(&t).unwrap();
        return state;
    }
}
#[derive(Serialize, Deserialize,Debug)]
struct CaseResult {
    id: i32,
    result: EnumResult,
    time: i32,
    memory: i32,
    info: String,
}
#[derive(Serialize, Deserialize,Debug)]
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
#[derive(Serialize, Deserialize,Debug)]
struct JobQuery{
    user_id:Option<i32>,
    user_name:Option<String>,
    contest_id:Option<i32>,
    problem_id:Option<i32>,
    language:Option<String>,
    from:Option<String>,
    to:Option<String>,
    state:Option<State>,
    result:Option<EnumResult>
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
async fn post_jobs(body: web::Json<Submit>, config: web::Data<Configure>,mut pool:web::Data<Pool<SqliteConnectionManager>>) -> impl Responder {
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
    } //TODO:用户id和比赛的检查
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
    let mut id=0;
    pool=pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row("SELECT count(*) FROM foo", [], |row| row.get(0))
        .unwrap();
    if conn!=0{
        let pool=pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row("SELECT max(id) FROM task", [], |row| row.get(0))
        .unwrap();
        id=(conn as i32)+1;
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
    pool=pool.clone();
    let mut conn=pool.get().unwrap().execute("INSERT INTO task VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
(&message.id,&message.submission.user_id,&String::new(),&message.submission.contest_id,&message.submission.problem_id,&message.submission.language,&message.created_time,message.state.to_string(),message.result.to_string(),&message.updated_time,&message.submission.source_code,&message.score,&serde_json::to_string(&message.cases).unwrap()));
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
    message.result = EnumResult::Running;
    message.cases[0].result = EnumResult::Running;
    message.updated_time=compile_time_start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    //update task TABLE
    pool=pool.clone();
    let conn=pool.get().unwrap().execute("UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5", ("Running".to_string(),"Running".to_string(),serde_json::to_string(&message.cases).unwrap(),message.updated_time.clone(),message.id));
    let status = Command::new(first_arg).args(args_vec).status();
    let updated_time = Utc::now();
    message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    if let Err(r) = status {        
        message.state = State::Finished;
        message.result = EnumResult::CompilationError;
        message.cases[0].result = EnumResult::CompilationError;        
        pool=pool.clone();
        let conn=pool.get().unwrap().execute("UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5", ("Finished".to_string(),"Compilation Error".to_string(),serde_json::to_string(&message.cases).unwrap(),message.updated_time.clone(),message.id));
        return HttpResponse::Ok().json(message);
    }
    message.cases[0].result = EnumResult::CompilationSuccess;
//upate task table
    pool=pool.clone();
    let conn=pool.get().unwrap().execute("UPDATE task SET cases=?1,updated_time=?2 WHERE id=?3", (serde_json::to_string(&message.cases).unwrap(),message.updated_time.clone(),message.id));
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
                    message.cases[index].result = EnumResult::RuntimeError;
                } else {
                    message.cases[index].result = match_result(
                        out_path,
                        config.problems[problem_index].cases[index]
                            .answer_file
                            .clone(),
                        config.problems[problem_index].ty.clone(),
                    );
                    if let EnumResult::Accepted=message.cases[index].result{
                        message.score+=config.problems[problem_index].cases[index].score;
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
        pool=pool.clone();
        let conn=pool.get().unwrap().execute("UPDATE task SET cases=?1,updated_time=?2,score=?3 WHERE id=?4", (serde_json::to_string(&message.cases).unwrap(),message.updated_time.clone(),message.score,message.id));
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
    pool=pool.clone();
    let conn=pool.get().unwrap().execute("UPDATE task SET state=?1,result=?2,updated_time=?3,score=?4 WHERE id=?5", ("Finished".to_string(),serde_json::to_string(&message.result).unwrap(),message.updated_time.clone(),message.score,message.id));
    std::fs::remove_dir_all(folder_name).unwrap();
    return HttpResponse::Ok().json(message);
}
#[get("/jobs")]
async fn get_jobs(info: web::Query<JobQuery>,config: web::Data<Configure>,mut pool:web::Data<Pool<SqliteConnectionManager>>)->impl Responder{
    let mut query_str=String::new();
    if let Some(s)=info.user_id{
        query_str.push_str(&format!("user_id={}",s));
    }if let Some(s)=&info.user_name{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }
            query_str.push_str(&format!("user_name={}",s));
    }if let Some(s)=info.contest_id{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("contest_id={}",s));
    }if let Some(s)=info.problem_id{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }query_str.push_str(&format!("problem_id={}",s));
    }if let Some(s)=&info.language{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }query_str.push_str(&format!("language={}",s));
    }if let Some(s)=&info.state{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }query_str.push_str(&format!("state={}",s.to_string()));
    }if let Some(s)=&info.result{
        if query_str.len()>1{
            query_str.push_str(" AND ");
        }query_str.push_str(&format!("result={}",s.to_string()));
    }if query_str.len()>1{
        let mut query_str=" WHERE ".to_string().push_str(&query_str);
    }pool=pool.clone();
    let mut conn=pool.get().unwrap();
    let mut t=conn.prepare(&format!("SELECT * FROM task{}",query_str)).unwrap();
    let tasks_iter = t.query_map([], |row| {
        Ok(Message {
            id: row.get(0)?,
            created_time: row.get(6)?,
            updated_time: row.get(9)?,
            submission:Submit { source_code: row.get(10)?, language: row.get(5)?, user_id: row.get(1)?, contest_id: row.get(3)?, problem_id: row.get(4)? },
            state:State::state_from_string(row.get(7).unwrap()),
            result:EnumResult::enumresult_from_string(row.get(8).unwrap()),
            score:row.get(11)?,
            cases:{
                let mut t:String=row.get(12)?;
                serde_json::from_str(&t).unwrap()
            }
        })
    }).unwrap();
    let mut vec_select=Vec::new();
    for i in tasks_iter{
        vec_select.push(i.unwrap());
    }
    //select from and to
    if let Some(r)=&info.from{
        let t=Utc.datetime_from_str(&r,"%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
        for mut i in 0..vec_select.len() as i32{
            let ti=Utc.datetime_from_str(&vec_select[i as usize].created_time,"%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
            let duration=ti.signed_duration_since(t);
            if let core::cmp::Ordering::Less=duration.cmp(&chrono::Duration::zero()){
                vec_select.remove(i as usize);
                i-=1;
            }
        }
    }
    if let Some(r)=&info.to{
        let t=Utc.datetime_from_str(&r,"%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
        for mut i in 0..vec_select.len() as i32{
            let ti=Utc.datetime_from_str(&vec_select[i as usize].created_time,"%Y-%m-%dT%H:%M:%S%.3fZ").unwrap();
            let duration=ti.signed_duration_since(t);
            if let core::cmp::Ordering::Greater=duration.cmp(&chrono::Duration::zero()){
                vec_select.remove(i as usize);
                i-=1;
            }
        }
    }
    return HttpResponse::Ok().json(vec_select);
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
    let server_address=match &config.server.bind_address{
        Some(r)=>{
            r.clone()
        }None=>{
            "127.0.0.1".to_string()
        }
    };
    let port_address=match config.server.bind_port{
        Some(r)=>{
            r
        }None=>{
            12345
        }
    };
    let manager=SqliteConnectionManager::file("file.db");
    let mut pool = r2d2::Pool::new(manager).unwrap();
    pool.get().unwrap().execute("
    CREATE TABLE IF NOT EXISTS task (
        id   INTEGER PRIMARY KEY,
        user_id INTEGER PRIMARY KEY,
        user_name TEXT,
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
    ",params![]).unwrap();
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
