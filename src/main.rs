use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use scraper::{ElementRef, Html, Selector};
use std::{collections::HashMap, fs::File, io};

use alpm::{Alpm, Ver};
use alpm_utils::{alpm_with_conf, config::Config, configure_alpm};

#[derive(Debug)]
struct OverviewRow {
    date: chrono::DateTime<FixedOffset>,
    title: String,
    link: String,
}

impl OverviewRow {
    fn new(date: chrono::DateTime<FixedOffset>, title: &str, link: &str) -> Self {
        Self {
            date,
            title: title.to_owned(),
            link: link.to_owned(),
        }
    }

    fn parse_row(element: ElementRef) -> Result<OverviewRow> {
        let td = Selector::parse("td").unwrap();
        let a = Selector::parse("a").unwrap();

        let tds = element.select(&td).collect::<Vec<ElementRef>>();
        if tds.len() > 1 {
            match tds[0..2] {
                [date, href] => {
                    let date = DateTime::parse_from_str(
                        &format!(
                            "{}T00:00:00-0000",
                            date.text().collect::<Vec<&str>>().join("")
                        ),
                        "%Y-%m-%dT%H:%M:%S%z",
                    )?;

                    let title = href.select(&a).collect::<Vec<ElementRef>>()[0]
                        .text()
                        .collect::<Vec<&str>>()
                        .join(" ");

                    let link = format!(
                        "https://archlinux.org{}",
                        href.select(&a).collect::<Vec<ElementRef>>()[0]
                            .attr("href")
                            .unwrap()
                    );

                    Ok(OverviewRow::new(date, &title, &link))
                }
                _ => Err(anyhow::anyhow!("Could not parse row.")),
            }
        } else {
            Err(anyhow::anyhow!("Row too short."))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::new()?;
    let local = alpm_with_conf(&config)?;

    let tmp = tempfile::tempdir().unwrap().into_path();
    let mut remote = Alpm::new("/", tmp.to_str().unwrap())?;
    configure_alpm(&mut remote, &config)?;

    remote.syncdbs_mut().update(true)?;

    let remote_pkgs = remote
        .syncdbs()
        .iter()
        .flat_map(|db| db.pkgs())
        .map(|pkg| (pkg.name(), pkg.version()))
        .collect::<HashMap<&str, &Ver>>();

    let updates = local
        .localdb()
        .pkgs()
        .iter()
        .map(|pkg| (pkg.name(), pkg.version()))
        .filter(|(name, version)| remote_pkgs.get(name) > Some(version))
        .collect::<HashMap<&str, &Ver>>();

    let log_file = io::read_to_string(File::open(local.logfile().unwrap())?)?;
    let lines = log_file.lines();
    let last_update = lines
        .into_iter()
        .rev()
        .find(|line| line.contains("pacman -Syu"))
        .map(|line| line.split_whitespace().collect::<Vec<&str>>()[0])
        .map(|time| time.trim_matches(|c| c == '[' || c == ']'))
        .map(|time| chrono::DateTime::parse_from_str(time, "%Y-%m-%dT%H:%M:%S%z"))
        .unwrap()?;

    let news = reqwest::get("https://archlinux.org/news")
        .await?
        .text()
        .await?;

    let news_html = Html::parse_document(&news);
    let article_tags = Selector::parse("tbody > tr").unwrap();

    let articles = news_html
        .select(&article_tags)
        .map(OverviewRow::parse_row)
        .filter_map(|row| row.ok())
        .filter(|article| article.date < last_update)
        .filter(|article| {
            updates
                .keys()
                .any(|s| article.title.to_lowercase().contains(&s.to_lowercase()))
        })
        .collect::<Vec<OverviewRow>>();

    println!(
        "Here are some news entries that relate to packages you plan on updating: {:#?}",
        articles
    );

    Ok(())
}
