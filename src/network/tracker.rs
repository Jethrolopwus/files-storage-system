use crate::core::{Config, Hash, PeerId, Statistics};
use anyhow::{Context, Result};
use log::{debug, error, info};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerEvent {
    Started,
    Stopped,
    Completed,
    None,
}

impl From<&str> for TrackerEvent {
    fn from(s: &str) -> Self {
        match s {
            "started" => TrackerEvent::Started,
            "stopped" => TrackerEvent::Stopped,
            "completed" => TrackerEvent::Completed,
            _ => TrackerEvent::None,
        }
    }
}

impl From<TrackerEvent> for &str {
    fn from(event: TrackerEvent) -> Self {
        match event {
            TrackerEvent::Started => "started",
            TrackerEvent::Stopped => "stopped",
            TrackerEvent::Completed => "completed",
            TrackerEvent::None => "",
        }
    }
}

//=== Tracker request parameters ===//
#[derive(Debug, Clone)]
pub struct TrackerRequest {
    pub info_hash: Hash,
    pub peer_id: PeerId,
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: TrackerEvent,
    pub compact: bool,
    pub numwant: Option<u32>,
    pub key: Option<String>,
    pub tracker_id: Option<String>,
}

impl TrackerRequest {
    pub fn new(
        info_hash: Hash,
        peer_id: PeerId,
        port: u16,
        uploaded: u64,
        downloaded: u64,
        left: u64,
        event: TrackerEvent,
    ) -> Self {
        Self {
            info_hash,
            peer_id,
            port,
            uploaded,
            downloaded,
            left,
            event,
            compact: true,
            numwant: Some(50),
            key: None,
            tracker_id: None,
        }
    }

    //=== Convert to URL query parameters ===//
    pub fn to_query_params(&self) -> String {
        let mut params = Vec::new();

        params.push(format!(
            "info_hash={}",
            urlencoding::encode_binary(&self.info_hash)
        ));
        params.push(format!(
            "peer_id={}",
            urlencoding::encode_binary(&self.peer_id)
        ));
        params.push(format!("port={}", self.port));
        params.push(format!("uploaded={}", self.uploaded));
        params.push(format!("downloaded={}", self.downloaded));
        params.push(format!("left={}", self.left));

        //=== Event parameter ===//
        if self.event != TrackerEvent::None {
            params.push(format!("event={}", <&str>::from(self.event)));
        }

        if self.compact {
            params.push("compact=1".to_string());
        }

        if let Some(numwant) = self.numwant {
            params.push(format!("numwant={}", numwant));
        }

        if let Some(ref key) = self.key {
            params.push(format!("key={}", key));
        }

        if let Some(ref tracker_id) = self.tracker_id {
            params.push(format!("trackerid={}", tracker_id));
        }

        params.join("&")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackerResponse {
    #[serde(rename = "failure reason")]
    pub failure_reason: Option<String>,
    #[serde(rename = "warning message")]
    pub warning_message: Option<String>,
    pub interval: Option<u32>,
    #[serde(rename = "min interval")]
    pub min_interval: Option<u32>,
    #[serde(rename = "tracker id")]
    pub tracker_id: Option<String>,
    pub complete: Option<u32>,
    pub incomplete: Option<u32>,
    pub peers: Option<Vec<PeerInfo>>,
    #[serde(rename = "peers6")]
    pub peers6: Option<Vec<PeerInfo>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PeerInfo {
    pub peer_id: Option<String>,
    pub ip: String,
    pub port: u16,
}

impl PeerInfo {
    pub fn to_socket_addr(&self) -> Result<SocketAddr> {
        let addr = format!("{}:{}", self.ip, self.port)
            .parse::<SocketAddr>()
            .with_context(|| format!("Failed to parse peer address: {}:{}", self.ip, self.port))?;
        Ok(addr)
    }
}

//=== Tracker client for communicating with BitTorrent trackers ===//
pub struct TrackerClient {
    config: Config,
    http_client: reqwest::Client,
}

impl TrackerClient {
    pub fn new(config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.tracker_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            http_client,
        }
    }

    //==== Announce to a tracker ====//
    pub async fn announce(
        &self,
        tracker_url: &str,
        request: &TrackerRequest,
    ) -> Result<TrackerResponse> {
        info!("Announcing to tracker: {}", tracker_url);

        //=== Build the URL ===//
        let mut url = Url::parse(tracker_url)
            .with_context(|| format!("Invalid tracker URL: {}", tracker_url))?;

        url.set_query(Some(&request.to_query_params()));

        debug!("Tracker request URL: {}", url);

        //=== Make the request ===//
        let response = timeout(
            self.config.tracker_timeout,
            self.http_client.get(url).send(),
        )
        .await
        .with_context(|| "Tracker request timeout")?
        .with_context(|| "Failed to send tracker request")?;

        //=== Check response status ===//
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Tracker request failed with status: {}",
                response.status()
            ));
        }
        //=== parse succee as text ===//
        let response_text = response
            .text()
            .await
            .with_context(|| "Failed to read tracker response")?;

        debug!("Tracker response: {}", response_text);

        //=== parse as JSON  ===//
        if let Ok(tracker_response) = serde_json::from_str::<TrackerResponse>(&response_text) {
            return Ok(tracker_response);
        }

        //=== parse as a simple bencode ===//
        self.parse_bencoded_response(&response_text)
    }

    //=== Parse bencoded tracker response instead of bencode library in real BitTorrent ===//
    fn parse_bencoded_response(&self, response_text: &str) -> Result<TrackerResponse> {
        if response_text.starts_with("d") && response_text.ends_with("e") {
            let mut response = TrackerResponse {
                failure_reason: None,
                warning_message: None,
                interval: Some(1800),
                min_interval: None,
                tracker_id: None,
                complete: None,
                incomplete: None,
                peers: Some(Vec::new()),
                peers6: None,
            };

            //=== Extract basic fields ===//
            if let Some(interval_start) = response_text.find("intervali") {
                if let Some(interval_end) = response_text[interval_start..].find("e") {
                    if let Ok(interval) = response_text
                        [interval_start + 9..interval_start + interval_end]
                        .parse::<u32>()
                    {
                        response.interval = Some(interval);
                    }
                }
            }

            //=== Extract simple peers ===//
            if let Some(_peers_start) = response_text.find("peers") {
                response.peers = Some(Vec::new());
            }

            Ok(response)
        } else {
            Err(anyhow::anyhow!("Invalid bencode response"))
        }
    }

    //=== Scrape tracker for torrent statistics ===//
    pub async fn scrape(
        &self,
        tracker_url: &str,
        info_hashes: &[Hash],
    ) -> Result<HashMap<Hash, ScrapeInfo>> {
        info!("Scraping tracker: {}", tracker_url);

        let mut url = Url::parse(tracker_url)
            .with_context(|| format!("Invalid tracker URL: {}", tracker_url))?;

        //=== Add info hashes to query ===//
        let info_hash_params: Vec<String> = info_hashes
            .iter()
            .map(|hash| format!("info_hash={}", urlencoding::encode_binary(hash)))
            .collect();

        url.set_query(Some(&info_hash_params.join("&")));

        let response = timeout(
            self.config.tracker_timeout,
            self.http_client.get(url).send(),
        )
        .await
        .with_context(|| "Tracker scrape timeout")?
        .with_context(|| "Failed to send tracker scrape request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Tracker scrape failed with status: {}",
                response.status()
            ));
        }

        //=== Parse text response ==//
        let response_text = response
            .text()
            .await
            .with_context(|| "Failed to read tracker scrape response")?;

        //=== Parse as JSON or bencode ===//
        if let Ok(scrape_response) =
            serde_json::from_str::<HashMap<String, ScrapeInfo>>(&response_text)
        {
            let mut result = HashMap::new();
            for (hash_str, scrape_info) in scrape_response {
                if let Ok(hash) = hex::decode(&hash_str) {
                    if hash.len() == 20 {
                        let mut hash_array = [0u8; 20];
                        hash_array.copy_from_slice(&hash);
                        result.insert(hash_array, scrape_info);
                    }
                }
            }
            Ok(result)
        } else {
            Ok(HashMap::new())
        }
    }
}

//=== Scrape information from tracker ===//
#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeInfo {
    pub complete: Option<u32>,
    pub downloaded: Option<u32>,
    pub incomplete: Option<u32>,
    pub name: Option<String>,
}

//=== Tracker manager for multiple trackers
pub struct TrackerManager {
    config: Config,
    tracker_client: TrackerClient,
    trackers: Vec<String>,
    last_announce: HashMap<String, Instant>,
    announce_intervals: HashMap<String, Duration>,
}

impl TrackerManager {
    pub fn new(config: Config, trackers: Vec<String>) -> Self {
        Self {
            tracker_client: TrackerClient::new(config.clone()),
            trackers,
            last_announce: HashMap::new(),
            announce_intervals: HashMap::new(),
            config,
        }
    }

    pub async fn announce_all(
        &mut self,
        info_hash: Hash,
        peer_id: PeerId,
        port: u16,
        statistics: &Statistics,
        event: TrackerEvent,
    ) -> Result<Vec<PeerInfo>> {
        let mut all_peers = Vec::new();

        let trackers = self.trackers.clone();
        for tracker_url in &trackers {
            match self
                .announce_to_tracker(tracker_url, info_hash, peer_id, port, statistics, event)
                .await
            {
                Ok(peers) => {
                    all_peers.extend(peers);
                    info!("Successfully announced to tracker: {}", tracker_url);
                }
                Err(e) => {
                    error!("Failed to announce to tracker {}: {}", tracker_url, e);
                }
            }
        }

        Ok(all_peers)
    }

    async fn announce_to_tracker(
        &mut self,
        tracker_url: &str,
        info_hash: Hash,
        peer_id: PeerId,
        port: u16,
        statistics: &Statistics,
        event: TrackerEvent,
    ) -> Result<Vec<PeerInfo>> {
        //=== Check to announce (respect intervals) ===//
        if let Some(last_announce) = self.last_announce.get(tracker_url) {
            if let Some(interval) = self.announce_intervals.get(tracker_url) {
                if last_announce.elapsed() < *interval {
                    debug!("Skipping announce to {} (too soon)", tracker_url);
                    return Ok(Vec::new());
                }
            }
        }

        //== Create request ==//
        let request = TrackerRequest::new(
            info_hash,
            peer_id,
            port,
            statistics.uploaded,
            statistics.downloaded,
            statistics.left,
            event,
        );

        //=== Send request ===//
        let response = self.tracker_client.announce(tracker_url, &request).await?;

        if let Some(failure_reason) = response.failure_reason {
            return Err(anyhow::anyhow!("Tracker failure: {}", failure_reason));
        }

        self.last_announce
            .insert(tracker_url.to_string(), Instant::now());

        if let Some(interval) = response.interval {
            self.announce_intervals.insert(
                tracker_url.to_string(),
                Duration::from_secs(interval as u64),
            );
        }

        //==== Extract peers ====//
        let mut peers = Vec::new();

        if let Some(peer_list) = response.peers {
            peers.extend(peer_list);
        }

        if let Some(peer_list) = response.peers6 {
            peers.extend(peer_list);
        }

        Ok(peers)
    }

    //=== Get trackers ===//
    pub fn trackers(&self) -> &[String] {
        &self.trackers
    }

    pub fn add_tracker(&mut self, tracker_url: String) {
        if !self.trackers.contains(&tracker_url) {
            self.trackers.push(tracker_url);
        }
    }
    pub fn remove_tracker(&mut self, tracker_url: &str) {
        self.trackers.retain(|t| t != tracker_url);
        self.last_announce.remove(tracker_url);
        self.announce_intervals.remove(tracker_url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_event_conversion() {
        assert_eq!(TrackerEvent::from("started"), TrackerEvent::Started);
        assert_eq!(TrackerEvent::from("stopped"), TrackerEvent::Stopped);
        assert_eq!(TrackerEvent::from("completed"), TrackerEvent::Completed);
        assert_eq!(TrackerEvent::from("unknown"), TrackerEvent::None);

        assert_eq!(<&str>::from(TrackerEvent::Started), "started");
        assert_eq!(<&str>::from(TrackerEvent::Stopped), "stopped");
        assert_eq!(<&str>::from(TrackerEvent::Completed), "completed");
        assert_eq!(<&str>::from(TrackerEvent::None), "");
    }

    #[test]
    fn test_tracker_request_creation() {
        let info_hash = [1u8; 20];
        let peer_id = [2u8; 20];
        let request = TrackerRequest::new(
            info_hash,
            peer_id,
            6881,
            1000,
            2000,
            3000,
            TrackerEvent::Started,
        );

        assert_eq!(request.info_hash, info_hash);
        assert_eq!(request.peer_id, peer_id);
        assert_eq!(request.port, 6881);
        assert_eq!(request.uploaded, 1000);
        assert_eq!(request.downloaded, 2000);
        assert_eq!(request.left, 3000);
        assert_eq!(request.event, TrackerEvent::Started);
    }

    #[test]
    fn test_tracker_request_query_params() {
        let info_hash = [1u8; 20];
        let peer_id = [2u8; 20];
        let request = TrackerRequest::new(
            info_hash,
            peer_id,
            6881,
            1000,
            2000,
            3000,
            TrackerEvent::Started,
        );

        let params = request.to_query_params();
        assert!(params.contains("info_hash="));
        assert!(params.contains("peer_id="));
        assert!(params.contains("port=6881"));
        assert!(params.contains("uploaded=1000"));
        assert!(params.contains("downloaded=2000"));
        assert!(params.contains("left=3000"));
        assert!(params.contains("event=started"));
    }

    #[tokio::test]
    async fn test_tracker_manager_creation() {
        let config = Config::default();
        let trackers = vec!["http://tracker.example.com/announce".to_string()];
        let manager = TrackerManager::new(config, trackers);

        assert_eq!(manager.trackers().len(), 1);
    }
}
