# rust-ForwardOptimal V2
rust-ForwardOptimal
这是一个高性能的 TCP 自动走低延迟 转发工具。它的核心逻辑是：“通过并发探测选出延迟最低的转发目标节点，并将流量实时转发过去”。

## 版本区别
### V1 介绍
##### 无脑选择最低延迟的节点 （那怕每10个包丢8个，只要在一瞬获得到最低延迟，就选最低延迟的）
##### 优点：快，监测一次，获得最低延迟目标，转发最低延迟目标
##### 缺点：不稳定，可能会选中丢包目标或高波动的目标

### V2 介绍
##### 智能优选：自动测量平均延迟、最低/最高延迟及丢包率。
##### 加入了 10次监测，最低延迟，最高延迟，综合延迟， 评分 (Score) 
##### 评分 = 平均延迟 + (丢包数 * 惩罚权重)。分值越低代表节点质量越好。
##### 获得10次延迟结果，并相加后除于10，获得综合延迟,优选综合延迟最低的目标进行转发 （丢包扣300分，视为300ms）
##### 优点：稳定，自动优选延迟稳定的目标
##### 其他：不一定走最低延迟，但确保目标的稳定性与低延迟。


```log
逻辑流程图
    启动 -> 加载 YAML。
    后台 -> [每隔 xx秒] -> 同时对所有目标发 TCP 握手 -> 选出 IP -> 更新内存状态。
    前端 -> [新连接进入] -> 读取内存状态 -> 连接最快 IP -> [可选] 发送 PPv2 头 -> 开始双向透传数据。
```



# 介绍
这一个非常简洁，高性能，基于rust，简单的TCP转发工具
它用于 定时检查多个 目标 ，然后获得最低延迟的目标
再对 最低延迟 的目标进行TCP转发



## 配置文件 YAML

```yaml

# 监听本地地址 (如V6可修改 ":::8080")
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

###  启动方式
```code

forward-optimal -c /root/config.yaml

```



### 其他（下载）

```shell
mkdir /etc/forward-optimal/
wget -P /etc/forward-optimal "https://github.com/yunbug/rust-ForwardOptimal/releases/download/v1.0.1/forward-optimal"
chmod 777 /etc/forward-optimal/forward-optimal

```

### （进程守护）
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
systemctl start forward-optimal.service

#查询
systemctl status forward-optimal.service

# 设置开机自启
systemctl enable forward-optimal

#重启 ！！！ 注意，每次修改配置文件都需要重启
systemctl restart forward-optimal

```

### 首先声明：代码没有完整的验证，其次想改什么自己改
