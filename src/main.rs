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

// 定义命令行参数结构

#[derive(Parser, Debug)]
#[command(name = "forward-optimal")]
#[command(author = "YY")]
#[command(version = "1.0.0")] 
#[command(about = "forward-optimal 高性能 TCP 最优路径转发工具,基于RUST开发", long_about = None)]

struct Args {
    /// 配置文件路径
    #[arg(short = 'c', long, default_value = "config.yaml")]
    config: String,

    /// 是否显示版本信息 (clap 默认支持 -V 和 --version，这里显式说明)
    #[arg(short = 'v', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
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

#[derive(Clone)]
struct BestTarget {
    addr: SocketAddr,
    name: String,
    rtt: Duration,
}

struct State {
    best: Option<BestTarget>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 解析命令行参数
    let args = Args::parse();

    // 初始化日志
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    // 2. 加载配置 (使用 args.config 指定的路径)
    let config_content = std::fs::read_to_string(&args.config)
        .with_context(|| format!("无法读取配置文件: {}", args.config))?;
    let config: Config = serde_yaml::from_str(&config_content)
        .with_context(|| "解析 YAML 失败")?;

    let state = Arc::new(RwLock::new(State { best: None }));

    // --- 后台探测任务 ---
    let state_clone = state.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        loop {
            log::info!("--------------------------------------------------");
            if let Some(best_node) = perform_parallel_check(&config_clone.targets).await {
                let mut s = state_clone.write().await;
                s.best = Some(best_node.clone());
                log::info!(">>> 最优节点: [{}] ({}) - {}ms", 
                    best_node.name, best_node.addr, best_node.rtt.as_millis());
            }
            log::info!("--------------------------------------------------");
            tokio::time::sleep(Duration::from_secs(config_clone.update_interval)).await;
        }
    });

    // --- 转发服务 ---
    let listener = TcpListener::bind(&config.bind_addr).await?;
    log::info!("服务启动，监听: {}，配置: {}", config.bind_addr, args.config);

    loop {
        let (client_stream, client_addr) = listener.accept().await?;
        let _ = client_stream.set_nodelay(true);

        let target_info = state.read().await.best.clone();
        if let Some(target) = target_info {
            let cfg = config.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_forward(client_stream, client_addr, target, cfg).await {
                    log::debug!("转发结束: {}", e);
                }
            });
        }
    }
}

async fn perform_parallel_check(targets: &[TargetConfig]) -> Option<BestTarget> {
    let tasks = targets.iter().map(|t| {
        let t = t.clone();
        async move {
            let addr = tokio::net::lookup_host(&t.addr).await.ok()?.next()?;
            let start = Instant::now();
            tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr)).await.ok()?.ok()?;
            let rtt = start.elapsed();
            log::info!("  [成功] 节点: {:<10} | 地址: {:<20} | 延迟: {}ms", t.name, addr, rtt.as_millis());
            Some(BestTarget { addr, name: t.name, rtt })
        }
    });
    let results = join_all(tasks).await;
    results.into_iter().flatten().min_by_key(|node| node.rtt)
}

async fn handle_forward(mut client: TcpStream, client_addr: SocketAddr, target: BestTarget, config: Config) -> Result<()> {
    let mut server = TcpStream::connect(target.addr).await?;
    let _ = server.set_nodelay(true);
    if let Some(ref proto) = config.proxy_protocol {
        if proto == "v2" {
            let header = build_proxy_v2_header(client_addr, target.addr);
            server.write_all(&header).await?;
        }
    }
    io::copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}

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