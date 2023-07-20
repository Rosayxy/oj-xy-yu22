extern crate reqwest;
use super::configure::{Language, Problem, ProblemType};
use super::enumresult::EnumResult;
use super::enumresult::State;
use actix_web::web;
use chrono::prelude::*;
use log;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use wait4::Wait4;
use wait_timeout::ChildExt;
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Submit {
    pub source_code: String,
    pub language: String,
    pub user_id: i32,
    pub contest_id: i32,
    pub problem_id: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseResult {
    pub id: i32,
    pub result: EnumResult,
    pub time: i32,
    pub memory: i32,
    pub info: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub id: i32,
    pub created_time: String,
    pub updated_time: String,
    pub submission: Submit,
    pub state: State,
    pub result: EnumResult,
    pub score: f64,
    pub cases: Vec<CaseResult>,
}
pub fn match_result(
    out_file: String,
    ans_file: String,
    compare: ProblemType,
) -> Result<EnumResult, std::io::Error> {
    let mut out_stream = File::open(out_file)?;
    let mut ans_stream = File::open(ans_file)?;
    let mut buffer_out = String::new();
    let mut buffer_ans = String::new();
    out_stream.read_to_string(&mut buffer_out)?;
    ans_stream.read_to_string(&mut buffer_ans)?;
    match compare {
        ProblemType::Strict => match buffer_out.cmp(&mut buffer_ans) {
            std::cmp::Ordering::Equal => {
                return Ok(EnumResult::Accepted);
            }
            _ => {
                return Ok(EnumResult::WrongAnswer);
            }
        },
        _ => {
            buffer_ans = buffer_ans.trim_end().to_string();
            buffer_out = buffer_out.trim_end().to_string();
            let vec_ans: Vec<_> = buffer_ans.split('\n').collect();
            let vec_out: Vec<_> = buffer_out.split('\n').collect();
            if vec_ans.len() != vec_out.len() {
                return Ok(EnumResult::WrongAnswer);
            }
            for i in 0..vec_ans.len() {
                match vec_ans[i].trim_end().cmp(&mut vec_out[i].trim_end()) {
                    std::cmp::Ordering::Equal => {
                        continue;
                    }
                    _ => {
                        return Ok(EnumResult::WrongAnswer);
                    }
                }
            }
            return Ok(EnumResult::Accepted);
        }
    }
}
pub fn match_result_spj(list: Vec<String>) -> (EnumResult, String) {
    let str1 = list[0].clone();
    let mut arg_list = Vec::new();
    for i in 1..list.len() {
        arg_list.push(list[i].clone());
    }
    let mut command = Command::new(str1);
    command.args(arg_list);
    let out = command.output();
    match out {
        Err(_r) => {
            return (EnumResult::SPJError, String::new());
        }
        Ok(s) => {
            if !s.status.success() {
                return (EnumResult::SPJError, String::from_utf8(s.stderr).unwrap());
            } else {
                let full_string = String::from_utf8(s.stdout).unwrap();
                let vec_str: Vec<&str> = full_string.split('\n').collect();
                if vec_str.len() <= 1 {
                    return (EnumResult::SPJError, String::new());
                }
                let mut parse_str = "\"".to_string();
                parse_str.push_str(vec_str[0]);
                parse_str.push('\"');
                let enum_res: Result<EnumResult, serde_json::Error> =
                    serde_json::from_str(&parse_str);
                match enum_res {
                    Ok(i) => {
                        return (i, vec_str[1].to_string());
                    }
                    Err(_r) => {
                        return (EnumResult::SPJError, vec_str[1].to_string());
                    }
                }
            }
        }
    }
}
pub fn execute_input_inner(
    mut message: Message,
    mut pool: web::Data<Pool<SqliteConnectionManager>>,
    problem: Problem,
    language: Language,
) -> Result<(), Box<dyn std::error::Error>> {
    let task_id = message.id;
    std::fs::create_dir(format!("temp{}", task_id))?;
    let folder_name = format!("temp{}", task_id);
    let src_path = format!("temp{}/{}", task_id, language.file_name);
    let mut buffer = std::fs::File::create(src_path.clone())?;
    buffer
        .write(message.submission.source_code.as_bytes())
        .unwrap();
    let mut args_vec = language.command.clone();
    for i in &mut args_vec {
        if i == "%OUTPUT%" {
            *i = format!("{}/test", folder_name);
        } else if i == "%INPUT%" {
            *i = src_path.clone();
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
    let _ = pool.get()?.execute(
        "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
        (
            "Running".to_string(),
            "Running".to_string(),
            serde_json::to_string(&message.cases)?,
            message.updated_time.clone(),
            message.id,
        ),
    );
    let status = Command::new(first_arg).args(args_vec).status();
    let updated_time = Utc::now();
    //update status
    message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    if let Err(_r) = status {
        message.state = State::Finished;
        message.result = EnumResult::CompilationError;
        message.cases[0].result = EnumResult::CompilationError;
        let _ = pool.get()?.execute(
            "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
            (
                "Finished".to_string(),
                "Compilation Error".to_string(),
                serde_json::to_string(&message.cases)?,
                message.updated_time.clone(),
                message.id,
            ),
        );
        std::fs::remove_dir_all(folder_name)?;
        return Ok(());
    } else if !status.unwrap().success() {
        message.state = State::Finished;
        message.result = EnumResult::CompilationError;
        message.cases[0].result = EnumResult::CompilationError;
        let _ = pool.get()?.execute(
            "UPDATE task SET state=?1,result=?2,cases=?3,updated_time=?4 WHERE id=?5",
            (
                "Finished".to_string(),
                "Compilation Error".to_string(),
                serde_json::to_string(&message.cases)?,
                message.updated_time.clone(),
                message.id,
            ),
        );
        std::fs::remove_dir_all(folder_name)?;
        return Ok(());
    }
    message.cases[0].result = EnumResult::CompilationSuccess;
    //update task table
    let _ = pool.get()?.execute(
        "UPDATE task SET cases=?1,updated_time=?2 WHERE id=?3",
        (
            serde_json::to_string(&message.cases)?,
            message.updated_time.clone(),
            message.id,
        ),
    );
    let execute_path = {
        let mut i = PathBuf::new();
        i.push(folder_name.clone());
        i.push("test".to_string());
        let i: PathBuf = i.iter().collect();
        i
    };
    let mut index = 1;
    //check if misc has packing
    let mut has_packing = false;
    let mut vec_packing: Vec<Vec<i32>> = Vec::new();
    if let Some(i) = &problem.misc {
        if let Some(r) = &i.packing {
            vec_packing = r.clone();
            has_packing = true;
        }
    }
    //start executing program
    for i in &problem.cases {
        if let EnumResult::Skipped = message.cases[index].result {
            index += 1;
            continue;
        }
        let out_path = {
            let mut i = PathBuf::new();
            i.push(folder_name.clone());
            i.push(format!("{}.out", index));
            let i: PathBuf = i.iter().collect();
            i
        };
        let out_file = File::create(&out_path)?;
        let in_file = File::open(&i.input_file)?;
        let mut child = Command::new(&execute_path)
            .stdin(Stdio::from(in_file))
            .stdout(Stdio::from(out_file))
            .spawn()?;
        let execute_start = Instant::now();
        let duration = Duration::from_micros(i.time_limit as u64);
        match child.wait_timeout(duration)? {
            Some(child_status) => {
                let execute_end = Instant::now();
                let t = execute_end.duration_since(execute_start);
                message.cases[index].time = t.as_micros() as i32;
                if !child_status.success() {
                    message.cases[index].result = EnumResult::RuntimeError;
                    if let EnumResult::Running=message.result{
                        message.result=EnumResult::RuntimeError;
                    }
                } else {
                    //if spj
                    if let ProblemType::Spj = problem.ty {
                        match &problem.misc {
                            None => {}
                            Some(i) => match &i.special_judge {
                                None => {}
                                Some(v) => {
                                    let mut vec_args = v.clone();
                                    for i in &mut vec_args {
                                        if i == "%OUTPUT%" {
                                            *i = out_path.to_str().unwrap().to_string();
                                        } else if i == "%ANSWER%" {
                                            *i = problem.cases[index - 1].answer_file.clone();
                                        }
                                    }
                                    (message.cases[index].result, message.cases[index].info) =
                                        match_result_spj(vec_args);
                                }
                            },
                        }
                    } else {
                        message.cases[index].result = match_result(
                            out_path.to_str().unwrap().to_string(),
                            problem.cases[index - 1].answer_file.clone(),
                            problem.ty.clone(),
                        )?;
                    }//assign message.result
                    if let EnumResult::Running=message.result{
                        if let EnumResult::Accepted=message.cases[index].result{
                        }else{
                            message.result=message.cases[index].result.clone();
                        }
                    }
                    if let EnumResult::Accepted = message.cases[index].result {
                        message.score += problem.cases[index - 1].score;
                    }
                }
            }
            None => {
                message.cases[index].time = i.time_limit;
                message.cases[index].result = EnumResult::TimeLimitExceeded;
                if let EnumResult::Running=message.result{
                    message.result=EnumResult::TimeLimitExceeded;
                }
                child.kill()?;
                child.wait()?;
            }
        }
        //if misc and not accepted,update skipped
        if has_packing {
            match message.cases[index].result {
                EnumResult::Accepted => {}
                _ => {
                    //locate index;
                    let mut row = 0;
                    let mut col = 0;
                    for i in 0..vec_packing.len() {
                        for j in 0..vec_packing[i].len() {
                            if vec_packing[i][j] as usize == index {
                                row = i;
                                col = j;
                                break;
                            }
                        }
                    }
                    //assign others in the pack status skipped
                    for i in col + 1..vec_packing[row].len() {
                        message.cases[vec_packing[row][i] as usize].result = EnumResult::Skipped;
                    }
                }
            }
        }
        index += 1;
        //update task TABLE
        let updated_time = Utc::now();
        message.updated_time = updated_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        pool = pool.clone();
        let _ = pool.get()?.execute(
            "UPDATE task SET cases=?1,updated_time=?2,score=?3 WHERE id=?4",
            (
                serde_json::to_string(&message.cases)?,
                message.updated_time.clone(),
                message.score,
                message.id,
            ),
        );
    }
    //update final result
    message.state = State::Finished;
    //patch has_packing
    if has_packing {
        for i in &vec_packing {
            match message.cases[i[i.len() - 1] as usize].result {
                EnumResult::Skipped => {
                    for j in 0..i.len() {
                        if let EnumResult::Accepted = message.cases[i[j] as usize].result {
                            message.score -= problem.cases[i[j] as usize - 1].score;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    //patch dynamic_ranking
    if let ProblemType::DynamicRanking = problem.ty {
        if let Some(r) = problem.misc {
            if let Some(f) = r.dynamic_ranking_ratio {
                message.score *= 1.0 - f;
            }
        }
    }
    message.updated_time = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut is_accepted = true;
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
            EnumResult::SPJError => {
                message.result = EnumResult::SPJError;
                is_accepted = false;
                break;
            }
            EnumResult::TimeLimitExceeded => {
                message.result = EnumResult::TimeLimitExceeded;
                is_accepted = false;
                break;
            }
            _ => {}
        }
    }
    if is_accepted {
        message.result = EnumResult::Accepted;
    }
    //update final result in task table
    pool = pool.clone();
    let _ = pool.get()?.execute(
        "UPDATE task SET state=?1,result=?2,updated_time=?3,score=?4 WHERE id=?5",
        (
            "Finished".to_string(),
            message.result.to_string(),
            message.updated_time.clone(),
            message.score,
            message.id,
        ),
    );
    std::fs::remove_dir_all(folder_name)?;
    Ok(())
}
pub fn execute_input(
    message: Message,
    pool: web::Data<Pool<SqliteConnectionManager>>,
    problem: Problem,
    language: Language,
) {
    let id = message.id;
    match execute_input_inner(message, pool.clone(), problem, language) {
        Ok(_ok) => {}
        Err(_r) => {
            pool.get()
                .unwrap()
                .execute(
                    "UPDATE task SET result=?1 WHERE id=?2",
                    (EnumResult::SystemError.to_string(), id),
                )
                .unwrap();
        }
    }
}
