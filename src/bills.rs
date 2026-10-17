//! Which Diet bill became an amending Act, from the House of Representatives bill database
//! compiled by smartnews-smri (MIT): https://github.com/smartnews-smri/house-of-representatives.
//! Only the bill's number and the URL of its text are used; the text itself is the House's
//! copyright and stays there.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use crate::cache::Cache;

const SOURCE: &str = "https://raw.githubusercontent.com/smartnews-smri/house-of-representatives/main/data/gian.json";

// e.g. "令和 8年 7月17日／56": promulgation date and law number
static PROMULGATED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(明治|大正|昭和|平成|令和)\s*(元|\d+)年\s*\d+月\s*\d+日／\s*(\d+)$").unwrap());

#[derive(Debug, PartialEq)]
pub struct Bill {
    pub kind: String,
    pub session: String,
    pub number: String,
    pub url: String,
}

/// Bills by (era digit of the e-Gov law ID, year, law number).
#[derive(Default)]
pub struct Bills(HashMap<(u32, u32, u32), Bill>);

fn era(name: &str) -> u32 {
    ["明治", "大正", "昭和", "平成", "令和"].iter().position(|e| *e == name).unwrap() as u32 + 1
}

fn path(cache: &Cache) -> PathBuf {
    cache.root.join("bills/gian.json")
}

impl Bills {
    pub fn parse(json: &[u8]) -> anyhow::Result<Self> {
        let rows: Vec<Vec<Value>> = serde_json::from_slice(json)?;
        let (header, rows) = rows.split_first().ok_or_else(|| anyhow::anyhow!("empty bill list"))?;
        let col = |name: &str| header.iter().position(|h| h == name).ok_or_else(|| anyhow::anyhow!("no column {name}"));
        let (kind, session, number, url, law) =
            (col("議案種類")?, col("提出回次")?, col("番号")?, col("本文情報URL")?, col("公布年月日／法律番号")?);
        let text = |row: &[Value], i: usize| row.get(i).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        let mut bills = HashMap::new();
        for row in rows {
            let Some(m) = PROMULGATED.captures(&text(row, law)) else { continue };
            let year = if &m[2] == "元" { 1 } else { m[2].parse()? };
            let bill = Bill { kind: text(row, kind), session: text(row, session), number: text(row, number), url: text(row, url) };
            bills.insert((era(&m[1]), year, m[3].parse()?), bill);
        }
        Ok(Bills(bills))
    }

    /// The bill list fetched last; empty when there is none.
    pub fn load(cache: &Cache) -> anyhow::Result<Self> {
        let path = path(cache);
        if !path.exists() {
            return Ok(Bills::default());
        }
        Bills::parse(&fs::read(path)?)
    }

    /// The bill behind an amending Act, by its e-Gov law ID (e.g. `508AC0000000056`).
    pub fn for_law(&self, law_id: &str) -> Option<&Bill> {
        let id = law_id.as_bytes();
        if id.len() < 7 || !id.is_ascii() || &law_id[3..5] != "AC" {
            return None;
        }
        let key = (law_id[..1].parse().ok()?, law_id[1..3].parse().ok()?, law_id[6..].parse().ok()?);
        self.0.get(&key)
    }
}

/// Refreshes the cached bill list; a failure keeps the previous one.
pub fn fetch(cache: &Cache) -> anyhow::Result<()> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
    let mut body = Vec::new();
    agent.get(SOURCE).call()?.into_reader().read_to_end(&mut body)?;
    Bills::parse(&body)?;
    let path = path(cache);
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn sample() -> Bills {
        let json = r#"[
            ["議案種類", "提出回次", "番号", "本文情報URL", "公布年月日／法律番号"],
            ["閣法", "221", "54", "https://example/g22109054.htm", "令和 8年 7月17日／56"],
            ["衆法", "139", "18", "https://example/g13905018.htm", "平成10年 3月25日／7"],
            ["参法", "201", "3", "https://example/g20103003.htm", "令和元年 6月 1日／1"],
            ["閣法", "222", "1", "", "／"]
        ]"#;
        Bills::parse(json.as_bytes()).unwrap()
    }

    #[test]
    fn finds_the_bill_of_an_amending_act() {
        let bills = sample();
        let got = bills.for_law("508AC0000000056").unwrap();
        assert_eq!((got.kind.as_str(), got.session.as_str(), got.number.as_str()), ("閣法", "221", "54"));
        assert_eq!(bills.for_law("410AC1000000007").unwrap().kind, "衆法");
        assert_eq!(bills.for_law("501AC1000000001").unwrap().kind, "参法");
    }

    #[test]
    fn only_acts_have_bills() {
        let bills = sample();
        assert!(bills.for_law("508CO0000000056").is_none());
        assert!(bills.for_law("508AC0000000057").is_none());
        assert!(bills.for_law("").is_none());
    }
}
