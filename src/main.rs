extern crate reqwest;
mod error_reason;
use error_reason::ErrorMessage;
use error_reason::ErrorReason;
mod enumresult;
use enumresult::EnumResult;
use enumresult::State;
mod configure;
use actix_web::rt::spawn;
use actix_web::web::block;
use actix_web::{
    get, middleware::Logger, post, put, web, App, HttpResponse, HttpServer, Responder,
    ResponseError,
};
use chrono::prelude::*;
use clap::Parser;
use configure::{get_configure, Configure};
use env_logger;
use log;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{Error, ErrorKind};
mod execute;
use execute::{execute_input, CaseResult, Message, Submit};
mod rank_sort;
use rank_sort::{
    sort_by_standard, RankRule, RanklistEntry, RanklistReturn, ScoringRule, ScoringRuleStandard,
    User,
};
use std::process::{Command, Stdio};
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

#[post("/jobs")]
async fn post_jobs(
    body: web::Json<Submit>,
    config: web::Data<Configure>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> Result<impl Responder, impl ResponseError> {
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
    for i in &config.languages {
        if i.name == body.language {
            is_language = true;
            break;
        }
        language_index += 1;
    }
    if is_problem == false {
        return Ok::<HttpResponse, ErrorMessage>(HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Problem {} not found.", body.problem_id),
        }));
    } else if is_language == false {
        return Ok(HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Language {} not found.", body.language),
        }));
    }
    //check user id
    pool = pool.clone();
    let mut conn: usize = 0;
    let conn1: Result<usize> = pool.get().map_err(ErrorMessage::r2error)?.query_row(
        "SELECT count(*) FROM users WHERE id=?1",
        params![body.user_id],
        |row| row.get(0),
    );
    match conn1 {
        Err(_r) => {
            return Ok(HttpResponse::InternalServerError().json(ErrorMessage {
                code: 5,
                reason: ErrorReason::ErrExternal,
                message: format!("Database query invalid"),
            }));
        }
        Ok(i) => {
            conn = i;
        }
    }
    if conn == 0 {
        return Ok(HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("User {} not found.", body.user_id),
        }));
    }
    //check contest_id
    let mut cnt: usize = 0;
    let conn1: Result<usize> = pool.get().map_err(ErrorMessage::r2error)?.query_row(
        "SELECT count(*) FROM users WHERE id=?1",
        params![body.user_id],
        |row| row.get(0),
    );
    match conn1 {
        Err(_r) => {
            return Ok(HttpResponse::InternalServerError().json(ErrorMessage {
                code: 5,
                reason: ErrorReason::ErrExternal,
                message: format!("Database query invalid"),
            }));
        }
        Ok(i) => {
            cnt = i;
        }
    }
    if cnt == 0 {
        return Ok(HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Contest {} not found.", body.contest_id),
        }));
    }
    //create contest element
    let conn = pool.get().map_err(ErrorMessage::r2error)?;
    let mut t = conn
        .prepare("SELECT * FROM contests WHERE id=?1")
        .map_err(ErrorMessage::rusqlite_error)?;
    let contests_iter = t
        .query_map(params![body.contest_id], |row| {
            Ok(Contest {
                id: row.get(0)?,
                name: row.get(1)?,
                from: row.get(2)?,
                to: row.get(3)?,
                problem_ids: {
                    let t: String = row.get(4)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                user_ids: {
                    let t: String = row.get(5)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                submission_limit: row.get(6)?,
            })
        })
        .unwrap();
    let mut vec_contest = Vec::new();
    for i in contests_iter {
        vec_contest.push(i?);
    } //check if user_id is in contest user_ids
    if (!vec_contest[0].user_ids.contains(&body.user_id)) && (body.contest_id != 0) {
        return Ok(HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: format!("user_id {} not found in contest.", body.user_id),
        }));
    }
    //check if problem_id is in contest problem_ids
    if !vec_contest[0].problem_ids.contains(&body.problem_id) && (body.contest_id != 0) {
        return Ok(HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: format!("problem_id {} not found in contest.", body.problem_id),
        }));
    }
    //check if in the allowed time
    let contest_from = Utc.datetime_from_str(&vec_contest[0].from, "%Y-%m-%dT%H:%M:%S%.3fZ")?;
    let contest_to = Utc.datetime_from_str(&vec_contest[0].to, "%Y-%m-%dT%H:%M:%S%.3fZ")?;
    if (Utc::now() > contest_to) || (Utc::now() < contest_from) {
        return Ok(HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: "submit time invalid".to_string(),
        }));
    } //check if in submit times
    let cnt: usize = pool
        .get()
        .map_err(ErrorMessage::r2error)?
        .query_row(
            "SELECT count(*) FROM task WHERE user_id=?1 AND contest_id=?2",
            params![body.user_id, body.contest_id],
            |row| row.get(0),
        )
        .map_err(ErrorMessage::rusqlite_error)?;
    if (cnt as i32) >= vec_contest[0].submission_limit {
        return Ok(HttpResponse::BadRequest().json(ErrorMessage {
            code: 4,
            reason: ErrorReason::ErrRateLimit,
            message: "exceed submission limit".to_string(),
        }));
    }
    log::info!("post_jobs_after check");
    //create temporary directory
    let mut vec_cases = Vec::new();
    for i in 0..config.problems[problem_index].cases.len() + 1 {
        vec_cases.push(CaseResult {
            id: i as i32,
            result: EnumResult::Waiting,
            time: 0,
            memory: 0,
            info: String::from(""),
        });
    }
    //determine job id
    let mut id = 0;
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .map_err(ErrorMessage::r2error)?
        .query_row("SELECT count(*) FROM task", [], |row| row.get(0))
        .map_err(ErrorMessage::rusqlite_error)?;
    if conn != 0 {
        let pool = pool.clone();
        let conn: usize = pool
            .get()
            .map_err(ErrorMessage::r2error)?
            .query_row("SELECT max(id) FROM task", [], |row| row.get(0))
            .map_err(ErrorMessage::rusqlite_error)?;
        id = (conn as i32) + 1;
    }
    let message = Message {
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
    let return_message = message.clone();
    //insert entry in task table
    pool = pool.clone();
    let _ = pool.get().map_err(ErrorMessage::r2error)?.execute(
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
            &serde_json::to_string(&message.cases)?,
        ),
    );
    //execute program
    let language = config.languages[language_index].clone();
    let problem = config.problems[problem_index].clone();
    let _detached = spawn(async {
        block(move || {
            let _ = execute_input(message, pool.clone(), problem, language);
        })
        .await
    });
    return Ok(HttpResponse::Ok().json(return_message));
}
#[get("/jobs")]
async fn get_jobs(
    info: web::Query<JobQuery>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    //create query-string
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
        query_str.push_str(&format!("language='{}'", s));
    }
    if let Some(s) = &info.state {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("state='{}'", s.to_string()));
    }
    if let Some(s) = &info.result {
        if query_str.len() > 1 {
            query_str.push_str(" AND ");
        }
        query_str.push_str(&format!("result='{}'", s.to_string()));
    }
    if query_str.len() > 1 {
        let mut t = " WHERE ".to_string();
        t.push_str(&query_str);
        query_str = t;
    }
    pool = pool.clone();
    let conn = pool.get().unwrap();
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
                    let t: String = row.get(11)?;
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
    let conn = pool.get().unwrap();
    let mut t = conn.prepare("SELECT * FROM task WHERE id=?1").unwrap();
    let tasks_iter = t
        .query_map([*id], |row| {
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
                    let t: String = row.get(11)?;
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
    //get message
    let conn = pool.get().unwrap();
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
                    let t: String = row.get(11)?;
                    let vec_cases: Vec<CaseResult> = serde_json::from_str(&t).unwrap();
                    vec_cases
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
      //get language
    let mut language_index = 0;
    for i in &config.languages {
        if i.name == message.submission.language {
            break;
        }
        language_index += 1;
    }
    let language = config.languages[language_index as usize].clone();
    //get problem
    let mut problem = config.problems[0].clone();
    for i in &config.problems {
        if i.id == message.submission.problem_id {
            problem = i.clone();
        }
    }
    //generate message return,update message
    let mut message_return = message.clone();
    message_return.result = EnumResult::Waiting;
    message_return.score = 0.0;
    message_return.state = State::Queueing;
    message_return.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    for i in &mut message_return.cases {
        i.result = EnumResult::Waiting;
        i.time = 0;
        i.memory = 0;
        i.info = String::new();
    }
    message.result = EnumResult::Waiting;
    message.score = 0.0;
    message.state = State::Queueing;
    message.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    for i in &mut message.cases {
        i.result = EnumResult::Waiting;
        i.time = 0;
        i.memory = 0;
        i.info = String::new();
    }
    pool.get().unwrap().execute(
        "UPDATE tasks SET updated_time=?1,state=?2,result=?3,score=?4,cases=?5 WHERE id=?6",
        params![
            &message.updated_time.clone(),
            message.state.to_string(),
            message.result.to_string(),
            &message.score,
            &serde_json::to_string(&message.cases).unwrap(),
            &message.id
        ],
    );
    //update sql
    let _ = spawn(async {
        block(move || {
            execute_input(message, pool.clone(), problem, language);
        })
        .await;
    });
    return HttpResponse::Ok().json(message_return);
}

#[derive(Serialize, Deserialize, Debug)]
struct InputUser {
    id: Option<i32>,
    name: String,
}
#[post("/users")]
async fn post_users(
    user: web::Json<InputUser>,
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
            let _ = pool
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
            println!("conn_for user_id max:{}", conn);
            user_id = conn + 1;
            println!("user_id:{}", user_id);
            pool = pool.clone();
            let _ = pool.get().unwrap().execute(
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

#[get("/contests/{contestid}/ranklist")]
async fn get_contest_ranklist(
    contestid: web::Path<i32>,
    rank_rule: web::Query<RankRule>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
    config: web::Data<Configure>,
) -> impl Responder {
    let mut vec_ranklist: Vec<RanklistEntry> = Vec::new();
    let mut vec_problem_id = Vec::new();
    //check if valid contest_id
    if contestid != 0.into() {
        let id = *contestid;
        let cnt: usize = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM contests WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        if cnt == 0 {
            return HttpResponse::NotFound().json(ErrorMessage {
                code: 3,
                reason: ErrorReason::ErrNotFound,
                message: format!("Contest {} not found.", id),
            });
        }
        //get contest info from table
        let conn = pool.get().unwrap();
        let mut t = conn.prepare("SELECT * FROM contests WHERE id=?1").unwrap();
        let contests_iter = t
            .query_map(params![*contestid], |row| {
                Ok(Contest {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    from: row.get(2)?,
                    to: row.get(3)?,
                    problem_ids: {
                        let t: String = row.get(4)?;
                        let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                        ids
                    },
                    user_ids: {
                        let t: String = row.get(5)?;
                        let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                        ids
                    },
                    submission_limit: row.get(6)?,
                })
            })
            .unwrap();
        let mut v = Vec::new();
        for i in contests_iter {
            v.push(i.unwrap());
        }
        vec_problem_id = v[0].problem_ids.clone();
        for i in &v[0].user_ids {
            let c1 = pool.get().unwrap();
            let mut conn = c1.prepare("SELECT * FROM users WHERE id=?1").unwrap();
            let users_iter = conn
                .query_map([*i], |row| {
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
        }
    } else {
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
        println!("vec_ranklist's length:{}", vec_ranklist.len());
        //get all problems's id
        for i in &config.problems {
            vec_problem_id.push(i.id);
        }
    }
    vec_problem_id.sort();
    for user in &mut vec_ranklist {
        for prob_id in &vec_problem_id {
            let c1 = pool.get().unwrap();
            let mut conn = c1
                .prepare("SELECT created_time,score FROM task WHERE user_id=?1 AND problem_id=?2")
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
            //if no submission found for that user and the problem
            if vec_submit.len() == 0 {
                println!("push when length is zero");
                user.scores.push(ScoringRuleStandard {
                    submit_time: None,
                    score: (0.0),
                });
            } else {
                //get the qualified one for the user and the problem
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
        }
        //update final score
        for i in &user.scores {
            user.final_score += i.score;
        }
    }
    //sort vec_ranklist
    vec_ranklist.sort_by(|a: &RanklistEntry, b| {
        match sort_by_standard(a, b, rank_rule.tie_breaker.clone(), pool.clone()) {
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
            Ordering::Equal => a.user.id.cmp(&b.user.id),
        }
    });
    //generate return list
    let mut vec_return = Vec::new();
    for i in 0..vec_ranklist.len() {
        if i == 0 {
            vec_return.push(RanklistReturn {
                user: vec_ranklist[i].user.clone(),
                rank: 1,
                scores: {
                    let mut vec_scores = Vec::new();
                    for it in &vec_ranklist[i].scores {
                        vec_scores.push(it.score);
                    }
                    vec_scores
                },
            });
        } else {
            let mut rank = 0;
            match sort_by_standard(
                &vec_ranklist[i - 1],
                &vec_ranklist[i],
                rank_rule.tie_breaker.clone(),
                pool.clone(),
            ) {
                Ordering::Equal => {
                    rank = vec_return[i - 1].rank;
                }
                _ => {
                    rank = (i + 1) as i32;
                }
            }
            vec_return.push(RanklistReturn {
                user: vec_ranklist[i].user.clone(),
                rank: rank,
                scores: {
                    let mut vec_scores = Vec::new();
                    for it in &vec_ranklist[i].scores {
                        vec_scores.push(it.score);
                    }
                    vec_scores
                },
            });
        }
    }
    return HttpResponse::Ok().json(vec_return);
}
#[derive(Serialize, Deserialize)]
struct ContestInput {
    id: Option<i32>,
    name: String,
    from: String,
    to: String,
    problem_ids: Vec<i32>,
    user_ids: Vec<i32>,
    submission_limit: i32,
}
#[derive(Serialize, Deserialize)]
struct Contest {
    id: i32,
    name: String,
    from: String,
    to: String,
    problem_ids: Vec<i32>,
    user_ids: Vec<i32>,
    submission_limit: i32,
}
#[post("/contests")]
async fn post_contest(
    contest: web::Json<ContestInput>,
    config: web::Data<Configure>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    //check id exists
    let mut id = 0;
    match contest.id {
        None => {
            let cnt: usize = pool
                .get()
                .unwrap()
                .query_row("SELECT count(*) FROM contests", params![], |row| row.get(0))
                .unwrap();
            if cnt == 0 {
                id = 1;
            } else {
                let conn: usize = pool
                    .get()
                    .unwrap()
                    .query_row("SELECT max(id) FROM contests", [], |row| row.get(0))
                    .unwrap();
                id = conn + 1;
            }
        }
        Some(i) => {
            if i == 0 {
                return HttpResponse::BadRequest().json(ErrorMessage {
                    code: 1,
                    reason: ErrorReason::ErrInvalidArgument,
                    message: "Invalid contest id".to_string(),
                });
            }
            let conn: usize = pool
                .get()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM contests WHERE id=?1",
                    params![i],
                    |row| row.get(0),
                )
                .unwrap();
            if conn == 0 {
                return HttpResponse::NotFound().json(ErrorMessage {
                    code: 3,
                    reason: ErrorReason::ErrNotFound,
                    message: format!("Contest {} not found.", i),
                });
            }
        }
    } //check if invalid
      //check if valid from and to time
    let from_time = Utc.datetime_from_str(&contest.from, "%Y-%m-%dT%H:%M:%S%.3fZ");
    if let Err(_r) = from_time {
        return HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: "Invalid argument from".to_string(),
        });
    }
    let to_time = Utc.datetime_from_str(&contest.to, "%Y-%m-%dT%H:%M:%S%.3fZ");
    if let Err(_r) = to_time {
        return HttpResponse::BadRequest().json(ErrorMessage {
            code: 1,
            reason: ErrorReason::ErrInvalidArgument,
            message: "Invalid argument to".to_string(),
        });
    }
    //check valid user_id
    //check if redundant user_id
    if contest.user_ids.len() > 1 {
        for i in 1..contest.user_ids.len() {
            for j in 0..i {
                if contest.user_ids[i] == contest.user_ids[j] {
                    return HttpResponse::BadRequest().json(ErrorMessage {
                        code: 1,
                        reason: ErrorReason::ErrInvalidArgument,
                        message: "Invalid argument user_ids".to_string(),
                    });
                }
            }
        }
    } //check if exists user_id
    for i in &contest.user_ids {
        pool = pool.clone();
        let t: usize = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM users WHERE id=?1",
                params![*i],
                |row| row.get(0),
            )
            .unwrap();
        if t == 0 {
            return HttpResponse::NotFound().json(ErrorMessage {
                code: 3,
                reason: ErrorReason::ErrNotFound,
                message: format!("user_id {} not found.", i),
            });
        }
    } //check if redundant problem_id
    if contest.problem_ids.len() > 1 {
        for i in 1..contest.problem_ids.len() {
            for j in 0..i {
                if contest.problem_ids[i] == contest.problem_ids[j] {
                    return HttpResponse::BadRequest().json(ErrorMessage {
                        code: 1,
                        reason: ErrorReason::ErrInvalidArgument,
                        message: "Invalid argument user_ids".to_string(),
                    });
                }
            }
        }
    } //check if exists problem_id
    let mut all_problemid_vec = Vec::new();
    for i in &config.problems {
        all_problemid_vec.push(i.id);
    }
    for i in &contest.problem_ids {
        let mut is_find = false;
        for j in &all_problemid_vec {
            if i == j {
                is_find = true;
                break;
            }
        }
        if !is_find {
            return HttpResponse::NotFound().json(ErrorMessage {
                code: 3,
                reason: ErrorReason::ErrNotFound,
                message: format!("problem_id {} not found.", i),
            });
        }
    }
    //new Contest and insert in table/update
    let contest_new = Contest {
        id: id as i32,
        name: contest.name.clone(),
        from: contest.from.clone(),
        to: contest.to.clone(),
        problem_ids: contest.problem_ids.clone(),
        user_ids: contest.user_ids.clone(),
        submission_limit: contest.submission_limit,
    };
    //update/new in table
    match contest.id {
        Some(_i) => {
            pool.get().unwrap().execute("UPDATE contests SET name=?1,from_time=?2,to_time=?3,problem_ids=?4,user_ids=?5,submission_limit=?6 WHERE id=?7", params![contest_new.name.clone(),contest_new.from.clone(),contest_new.to.clone(),serde_json::to_string(&contest_new.problem_ids).unwrap(),serde_json::to_string(&contest_new.user_ids).unwrap(),contest_new.submission_limit,id]).unwrap();
        }
        None => {
            pool.get()
                .unwrap()
                .execute(
                    "INSERT INTO contests VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    (
                        id,
                        contest_new.name.clone(),
                        contest_new.from.clone(),
                        contest_new.to.clone(),
                        serde_json::to_string(&contest_new.problem_ids).unwrap(),
                        serde_json::to_string(&contest_new.user_ids).unwrap(),
                        contest_new.submission_limit,
                    ),
                )
                .unwrap();
        }
    }
    return HttpResponse::Ok().json(contest_new);
}
#[get("/contests")]
async fn get_contests(pool: web::Data<Pool<SqliteConnectionManager>>) -> impl Responder {
    let conn = pool.get().unwrap();
    let mut t = conn.prepare("SELECT * FROM contests").unwrap();
    let contests_iter = t
        .query_map([], |row| {
            Ok(Contest {
                id: row.get(0)?,
                name: row.get(1)?,
                from: row.get(2)?,
                to: row.get(3)?,
                problem_ids: {
                    let t: String = row.get(4)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                user_ids: {
                    let t: String = row.get(5)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                submission_limit: row.get(6)?,
            })
        })
        .unwrap();
    let mut vec_return = Vec::new();
    for i in contests_iter {
        let t = i.unwrap();
        if t.id != 0 {
            vec_return.push(t);
        }
    }
    vec_return.sort_by(|a, b| a.id.cmp(&b.id));
    return HttpResponse::Ok().json(vec_return);
}
#[get("/contests/{id}")]
async fn get_contest_id(
    id: web::Path<i32>,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
) -> impl Responder {
    pool = pool.clone();
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM users WHERE id=?1",
            params![*id],
            |row| row.get(0),
        )
        .unwrap();
    if conn == 0 {
        return HttpResponse::NotFound().json(ErrorMessage {
            code: 3,
            reason: ErrorReason::ErrNotFound,
            message: format!("Contest {} not found.", id),
        });
    }
    let conn = pool.get().unwrap();
    let mut t = conn.prepare("SELECT * FROM contests WHERE id=?1").unwrap();
    let id_i32: i32 = *id;
    let contests_iter = t
        .query_map(params![id_i32], |row| {
            Ok(Contest {
                id: row.get(0)?,
                name: row.get(1)?,
                from: row.get(2)?,
                to: row.get(3)?,
                problem_ids: {
                    let t: String = row.get(4)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                user_ids: {
                    let t: String = row.get(5)?;
                    let ids: Vec<i32> = serde_json::from_str(&t).unwrap();
                    ids
                },
                submission_limit: row.get(6)?,
            })
        })
        .unwrap();
    let mut v = Vec::new();
    for i in contests_iter {
        v.push(i.unwrap());
    }
    return HttpResponse::Ok().json(v.pop().unwrap());
}
fn create_contest0(config: &Configure, pool: Pool<SqliteConnectionManager>) {
    let conn: usize = pool
        .get()
        .unwrap()
        .query_row("SELECT count(*) FROM contests WHERE id=0", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut users: Vec<i32> = Vec::new();
    let mut problems = Vec::new();
    for i in &config.problems {
        problems.push(i.id);
    }
    let pool_binding = pool.get().unwrap();
    let mut users_prepare = pool_binding.prepare("SELECT id FROM users").unwrap();
    let users_iter = users_prepare
        .query_map(params![], |row| row.get(0))
        .unwrap();
    for i in users_iter {
        users.push(i.unwrap());
    }
    let from_time = "1022-08-27T02:05:29.000Z".to_string();
    let to_time = "3022-08-27T02:05:30.000Z".to_string();
    if conn != 0 {
        let _ = pool.get().unwrap().execute(
            "UPDATE contests SET problem_ids=?1,user_ids=?2",
            params![
                serde_json::to_string(&problems).unwrap(),
                serde_json::to_string(&users).unwrap()
            ],
        );
    } else {
        let _ = pool.get().unwrap().execute(
            "INSERT INTO contests VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                0,
                "root".to_string(),
                from_time,
                to_time,
                serde_json::to_string(&problems).unwrap(),
                serde_json::to_string(&users).unwrap(),
                1 << 30
            ],
        );
    }
}
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    //remove all the past temporary directories
    let ls = Command::new("ls").output().unwrap();
    let out_ls = ls.stdout.as_slice();
    let vec_dir = String::from_utf8(out_ls.to_vec()).unwrap();
    let vec_str: Vec<&str> = vec_dir.split_ascii_whitespace().collect();
    for i in vec_str {
        if i.contains("temp") {
            std::fs::remove_dir_all(i.to_string());
        }
    }
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
    //init sql
    let server_address = match &config.server.bind_address {
        Some(r) => r.clone(),
        None => "127.0.0.1".to_string(),
    };
    let port_address: u16 = match config.server.bind_port {
        Some(r) => r,
        None => 12345,
    };
    //remove database
    if cli.flush_data {
        let _ = std::fs::remove_file("file.db");
    }
    let manager = SqliteConnectionManager::file("file.db");
    let mut pool = r2d2::Pool::new(manager).unwrap();
    pool.get()
        .unwrap()
        .execute(
            "
    CREATE TABLE IF NOT EXISTS task(
        id   INTEGER PRIMARY KEY,
        user_id INTEGER,
        contest_id INTEGER,
        problem_id INTEGER,
        language TEXT NOT NULL,
        created_time TEXT NOT NULL,
        state TEXT NOT NULL,
        result TEXT NOT NULL,
        updated_time TEXT NOT NULL,
        source_code TEXT NOT NULL,
        score REAL,
        cases TEXT NOT NULL
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
    CREATE TABLE IF NOT EXISTS users(
        id INTERGER PRIMARY KEY,
        name TEXT NOT NULL
    )
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
        let _ = pool
            .get()
            .unwrap()
            .execute("INSERT INTO users (id, name) VALUES (?1, ?2)", (0, "root"));
    }
    pool.get()
        .unwrap()
        .execute(
            "CREATE TABLE IF NOT EXISTS contests(
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        from_time TEXT NOT NULL,
        to_time TEXT NOT NULL,
        problem_ids TEXT NOT NULL,
        user_ids TEXT NOT NULL,
        submission_limit INTEGER
    )",
            params![],
        )
        .unwrap();
    create_contest0(&config, pool.clone());
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(pool.clone()))
            .wrap(Logger::default())
            .route("/hello", web::get().to(|| async { "Hello World!" }))
            .service(greet)
            // DO NOT REMOVE: used in automatic testing
            .service(post_jobs)
            .service(get_jobs)
            .service(gets_job_id)
            .service(put_jobs)
            .service(post_users)
            .service(get_users)
            .service(post_contest)
            .service(get_contest_id)
            .service(get_contests)
            .service(get_contest_ranklist)
            .service(exit)
    })
    .bind((server_address.as_str(), port_address))?
    .run()
    .await
}
