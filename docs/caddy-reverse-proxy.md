## Caddy

```
# Caddyfile
example.com {
        tls foo@bar.com
        
        reverse_proxy http://127.0.0.1:8080 {
                header_up Host {upstream_hostport}
                header_up X-Real-IP {http.request.X-Real-IP}                    # or {remote_host}
                header_up X-Forwarded-For {http.request.header.X-Forwarded-For} # or {remote_host}
        }
}
```
