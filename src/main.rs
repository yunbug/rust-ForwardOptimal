use anyhow::{Context, Result};
use clap::Parser;
use futures::future::join_all;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(name = "forward-optimal", version = "2.0.1", about = "TCP 最优路径转发")]
struct Args {
    #[arg(short = 'c', long, default_value = "config.yaml")]
    config: String,
}

#[derive(Debug, Deserialize, Clone)]
struct TargetConfig {
    name: String,
    addr: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    bind_addr: String,
    targets: Vec<TargetConfig>,
    update_interval: u64,
    proxy_protocol: Option<String>,
}

#[derive(Clone, Debug)]
struct BestTarget {
    addr: SocketAddr,
    name: String,
    score: u128,
}

struct State {
    best: Option<BestTarget>,
}

// --- 配置参数 ---
const PROBE_COUNT: u32 = 10;       // 每轮探测次数
const PENALTY_MS: u128 = 150;      // 失败惩罚分 (丢包权重)
const CONNECT_TIMEOUT: u64 = 1000; // (1000ms)1秒连接超时

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // 初始化日志
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::builder()
        .format_target(false)
        .format_timestamp_secs()
        .init();

    let config_content = std::fs::read_to_string(&args.config)
        .with_context(|| format!("无法读取配置文件: {}", args.config))?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let state = Arc::new(RwLock::new(State { best: None }));

    // --- 后台探测任务 ---
    let state_clone = state.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        loop {
            log::info!("--- 正在探测节点状态 ---");

            if let Some(winner) = perform_scoring_check(&config_clone.targets).await {
                let mut s = state_clone.write().await;
                
                let changed = s.best.as_ref().map(|b| b.name != winner.name).unwrap_or(true);
                if changed {
                    log::info!(">>> 路由切换: 选定最优节点 [{}] ({})", winner.name, winner.addr);
                }
                
                s.best = Some(winner);
            } else {
                log::warn!("!!! 本轮探测没有发现任何可用节点");
            }
            
            tokio::time::sleep(Duration::from_secs(config_clone.update_interval)).await;
        }
    });

    // --- 监听服务 ---
    let listener = TcpListener::bind(&config.bind_addr).await?;
    log::info!("服务启动: {} (优选间隔: {}秒)", config.bind_addr, config.update_interval);

    loop {
        let (client_stream, _) = listener.accept().await?;
        let target_info = state.read().await.best.clone();
        
        if let Some(target) = target_info {
            let cfg = config.clone();
            tokio::spawn(async move {
                if let Ok(addr) = client_stream.peer_addr() {
                    log::info!("转发请求: {} -> [{}]", addr, target.name);
                    let _ = handle_forward(client_stream, target, cfg).await;
                }
            });
        }
    }
}

/// 执行评分探测 (按指定中文格式输出)
async fn perform_scoring_check(targets: &[TargetConfig]) -> Option<BestTarget> {
    let tasks = targets.iter().map(|t| {
        let t = t.clone();
        async move {
            let addr = match tokio::net::lookup_host(&t.addr).await {
                Ok(mut addrs) => addrs.next()?,
                Err(_) => {
                    log::warn!("[{}] DNS解析失败", t.name);
                    return None;
                }
            };

            let mut valid_rtt_sum: u128 = 0;
            let mut success_count = 0;
            let mut min_ms: u128 = u128::MAX;
            let mut max_ms: u128 = 0;

            for _ in 0..PROBE_COUNT {
                let start = Instant::now();
                let res = tokio::time::timeout(
                    Duration::from_millis(CONNECT_TIMEOUT),
                    TcpStream::connect(addr)
                ).await;

                if let Ok(Ok(_)) = res {
                    let rtt = start.elapsed().as_millis();
                    success_count += 1;
                    valid_rtt_sum += rtt;
                    
                    if rtt < min_ms { min_ms = rtt; }
                    if rtt > max_ms { max_ms = rtt; }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            if success_count == 0 {
                log::error!("[{}] ({}) 评分: INF (无法连接, 100% 丢包)", t.name, addr);
                None
            } else {
                let fail_count = PROBE_COUNT - success_count;
                // Score 计算公式: (响应时间总和 + 丢包数*惩罚) / 总次数
                let final_score = (valid_rtt_sum + (fail_count as u128 * PENALTY_MS)) / PROBE_COUNT as u128;
                let avg_ms = valid_rtt_sum / success_count as u128;

                // 按照要求的格式打印日志
                log::info!(
                    "[{}] ({}) 评分: {} (最低延迟: {}, 最高延迟: {}, 平均延迟: {}, 丢包: {}/{})", 
                    t.name, 
                    addr, 
                    final_score, 
                    min_ms, 
                    max_ms, 
                    avg_ms, 
                    fail_count, 
                    PROBE_COUNT
                );

                Some(BestTarget { addr, name: t.name, score: final_score })
            }
        }
    });

    let results = join_all(tasks).await;
    results.into_iter().flatten().min_by_key(|n| n.score)
}

/// 转发逻辑
async fn handle_forward(mut client: TcpStream, target: BestTarget, config: Config) -> Result<()> {
    let mut server = TcpStream::connect(target.addr).await?;
    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);

    if let Some(ref proto) = config.proxy_protocol {
        if proto == "v2" {
            if let Ok(src_addr) = client.peer_addr() {
                let header = build_proxy_v2_header(src_addr, target.addr);
                server.write_all(&header).await?;
            }
        }
    }

    io::copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}

/// PROXY Protocol V2 构造器
fn build_proxy_v2_header(src: SocketAddr, dst: SocketAddr) -> Vec<u8> {
    let mut header = Vec::with_capacity(32);
    header.extend_from_slice(b"\x0D\x0A\x0D\x0A\x00\x0D\x0A\x51\x55\x49\x54\x0A");
    header.push(0x21); 
    match (src, dst) {
        (SocketAddr::V4(s), SocketAddr::V4(d)) => {
            header.push(0x11);
            header.extend_from_slice(&12u16.to_be_bytes());
            header.extend_from_slice(&s.ip().octets());
            header.extend_from_slice(&d.ip().octets());
            header.extend_from_slice(&s.port().to_be_bytes());
            header.extend_from_slice(&d.port().to_be_bytes());
        }
        (SocketAddr::V6(s), SocketAddr::V6(d)) => {
            header.push(0x21);
            header.extend_from_slice(&36u16.to_be_bytes());
            header.extend_from_slice(&s.ip().octets());
            header.extend_from_slice(&d.ip().octets());
            header.extend_from_slice(&s.port().to_be_bytes());
            header.extend_from_slice(&d.port().to_be_bytes());
        }
        _ => {
            header.push(0x00);
            header.extend_from_slice(&0u16.to_be_bytes());
        }
    }
    header
}
