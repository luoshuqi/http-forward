# http-forward

#### 介绍
HTTP 转发工具，可用来做内网穿透。

#### 软件架构
分为客户端和服务端。

客户端通过 TCP 连接到服务端，声明要转发的域名，保持连接打开。

服务端收到 HTTP 请求后，解析 `Host` 头获取域名（只解析 `Host` 头，不解析完整请求，并且每个 TCP 连接只解析一次），然后通知对应的客户端。
客户端收到消息后，另外建立一个到服务端的连接，服务端把这个连接和 HTTP 连接关联起来。

#### 构建

```shell
cargo build --release
```

`bin` 目录有编译好的适用于 `x86_64 linux` 的可执行文件。

#### 用法

客户端：
```shell
USAGE:
    http_forward_client [OPTIONS] --client-cert <client-cert> --client-key <client-key> --server-addr <server-addr>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -c, --client-cert <client-cert>    客户端证书
    -k, --client-key <client-key>      客户端证书 key
    -f, --forward <forward>...         转发配置，格式为"域名:转发地址"。示例："a.foo.com:127.0.0.1:80" 表示把对
                                       a.foo.com 的请求转发到127.0.0.1:80
    -s, --server-addr <server-addr>    服务器地址, 格式为"域名:端口"
```

服务端：
```shell
USAGE:
    http_forward_server --addr <addr> --http-addr <http-addr> --http-cert <http-cert> --http-key <http-key> --server-cert <server-cert> --server-key <server-key>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --addr <addr>                  绑定地址，格式为 "ip:端口"
        --http-addr <http-addr>        http 绑定地址，格式为 "ip:端口"
        --http-cert <http-cert>        http 证书
        --http-key <http-key>          http 证书 key
        --server-cert <server-cert>    服务端证书
        --server-key <server-key>      服务端证书 key
```

#### 关于证书

服务端客户端做 SSL 双向认证，服务端只会接受使用了由服务端证书签发的证书的客户端。

使用 `cert.sh` 生成服务端客户端证书。
```shell
bash cert.sh <域名>
```

由于需要解析 `Host` 头，https 连接必须与转发服务端建立，所以需要 http 证书。

