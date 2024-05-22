## Caddy

```nginx
server {
  listen 443 ssl;
  listen [::]:443 ssl;

  location / {
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $http_x_real_ip;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_pass http://127.0.0.1:8080/;
  }
}
```
