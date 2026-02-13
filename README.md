# rust-ForwardOptimal
rust-ForwardOptimal
这是一个高性能的 TCP 自动走低延迟 转发工具。它的核心逻辑是：“通过并发探测选出延迟最低的转发目标节点，并将流量实时转发过去”。

逻辑流程图
    启动 -> 加载 YAML。
    后台 -> [每隔 60秒] -> 同时对所有目标发 TCP 握手 -> 选出最快 IP -> 更新内存状态。
    前端 -> [新连接进入] -> 读取内存状态 -> 连接最快 IP -> [可选] 发送 PPv2 头 -> 开始双向透传数据。

配置文件 YAML

# 介绍
这一个非常简洁，高性能，基于rust，简单的TCP转发工具
它用于 定时检查多个 目标 ，然后获得最低延迟的目标
再对 最低延迟 的目标进行TCP转发




```yaml

# 监听本地地址
bind_addr: "0.0.0.0:8080"

# 检测间隔（秒）
update_interval: 60

# 是否开启 Proxy Protocol (可选: "v2" 或留空)
proxy_protocol: ""

# 目标服务器列表
targets:
  - name: "Cloudflare"
    addr: "1.1.1.1:80"
  - name: "DNS"
    addr: "8.8.8.8:80"
  - name: "IPV6-VPS-1"
    addr: "[2607:f8b0:400a:80c::200e]:443"

```

##  启动方式
```code

forward-optimal -c /root/config.yaml

```

### 其他（进程守护）
```code
echo ' 
[Unit]
Description=forward-optimal
After=network.target
Wants=network.target

[Service]
User=root
Group=root
Type=simple
LimitAS=infinity
LimitRSS=infinity
LimitCORE=infinity
LimitNOFILE=999999999
WorkingDirectory=/etc/forward-optimal/
ExecStart=/etc/forward-optimal/forward-optimal -c /root/config.yaml
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
' >/etc/systemd/system/forward-optimal.service
```

```code
# 初始化
systemctl daemon-reload

# 启动
systemctl start ForwardOptimal.service

#查询
systemctl status ForwardOptimal.service

# 设置开机自启
systemctl enable ForwardOptimal

#重启 ！！！ 注意，每次修改配置文件都需要重启
systemctl restart ForwardOptimal

```

### 首先声明：代码没有完整的验证，其次想改什么自己改
