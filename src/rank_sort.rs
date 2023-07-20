use actix_web::web;
use chrono::{DateTime, TimeZone, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub id: i32,
    pub name: String,
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ScoringRule {
    Latest,
    Highest,
}
#[derive(Clone, Debug)]
pub struct ScoringRuleStandard {
    pub submit_time: Option<DateTime<Utc>>,
    pub score: f64,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum TieBreaker {
    SubmissionTime,
    SubmissionCount,
    UserId,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct RankRule {
    pub scoring_rule: Option<ScoringRule>,
    pub tie_breaker: Option<TieBreaker>,
}
#[derive(Debug)]
pub struct RanklistEntry {
    pub user: User,
    pub rank: i32,
    pub final_score: f64,
    pub scores: Vec<ScoringRuleStandard>,
}
#[derive(Serialize)]
pub struct RanklistReturn {
    pub user: User,
    pub rank: i32,
    pub scores: Vec<f64>,
}
pub fn sort_by_standard(
    a: &RanklistEntry,
    b: &RanklistEntry,
    standard: Option<TieBreaker>,
    pool: web::Data<Pool<SqliteConnectionManager>>,
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
                        "SELECT count(*) FROM task WHERE user_id=?1",
                        params![a.user.id],
                        |row| row.get(0),
                    )
                    .unwrap();
                let connb: usize = pool
                    .get()
                    .unwrap()
                    .query_row(
                        "SELECT count(*) FROM task WHERE user_id=?1",
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
            }
            TieBreaker::UserId => {
                if a.final_score > b.final_score {
                    return Ordering::Less;
                }
                if a.final_score < b.final_score {
                    return Ordering::Greater;
                } else {
                    return a.user.id.cmp(&b.user.id);
                }
            }
            TieBreaker::SubmissionTime => {
                if a.final_score > b.final_score {
                    return Ordering::Less;
                }
                if a.final_score < b.final_score {
                    return Ordering::Greater;
                } else {
                    let mut submit_time_a: DateTime<Utc> = Utc
                        .datetime_from_str("1022-08-27T02:05:29.000Z", "%Y-%m-%dT%H:%M:%S%.3fZ")
                        .unwrap();
                    let mut submit_time_b: DateTime<Utc> = submit_time_a.clone();
                    for i in &a.scores {
                        match i.submit_time {
                            Some(t) => {
                                if t > submit_time_a {
                                    submit_time_a = t.clone();
                                }
                            }
                            None => {}
                        }
                    }
                    if submit_time_a
                        == Utc
                            .datetime_from_str("1022-08-27T02:05:29.000Z", "%Y-%m-%dT%H:%M:%S%.3fZ")
                            .unwrap()
                    {
                        submit_time_a = Utc
                            .datetime_from_str("4022-08-27T02:05:29.000Z", "%Y-%m-%dT%H:%M:%S%.3fZ")
                            .unwrap();
                    }
                    for i in &b.scores {
                        match i.submit_time {
                            Some(t) => {
                                if t > submit_time_b {
                                    submit_time_b = t.clone();
                                }
                            }
                            None => {}
                        }
                    }
                    if submit_time_b
                        == Utc
                            .datetime_from_str("1022-08-27T02:05:29.000Z", "%Y-%m-%dT%H:%M:%S%.3fZ")
                            .unwrap()
                    {
                        submit_time_b = Utc
                            .datetime_from_str("4022-08-27T02:05:29.000Z", "%Y-%m-%dT%H:%M:%S%.3fZ")
                            .unwrap();
                    }
                    if submit_time_a < submit_time_b {
                        return Ordering::Less;
                    } else if submit_time_a > submit_time_b {
                        return Ordering::Greater;
                    } else {
                        return Ordering::Equal;
                    }
                }
            }
        },
    }
}
