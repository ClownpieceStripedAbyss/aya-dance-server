use std::collections::HashMap;

use clap::Parser;
use log::{info, warn};
use wanna_cdn::{AppOpts, AppServiceImpl};

fn print_license() {
  println!(
    r#"
# 使用条款 Term of Use

使用本程序即表示您同意以下条款：
1. 本程序的允许使用范围：
   1. 本程序不得用于任何违法、违规、违背道德、攻击、侵犯他人权益、破坏国家安全等行为。
   2. 本程序不得用于任何商业、盈利、广告等环境。
   3. 本程序不得用于任何违反 [VRChat] 官方规定的行为。
   4. 本程序仅限于在 [VRChat] 的 [WannaDance] 及 [WannaDance Dev] 地图中使用，不得用于其他地图。
   5. 本程序仅限于在个人使用的电脑上使用，不得用于任何具有服务器、云服务器、SaaS 属性的环境。
   6. 本程序仅限于在个人及家庭环境中使用，不得用于任何公共场所、公共网络、公开提供服务等环境。
   7. 任何上述没有提及的允许使用范围，均应解释为不允许使用。
   8. 任何因为不在允许范围内使用本程序导致的任何问题，由实际使用者承担，WannaDance 团队概不负责，也不对此情景提供任何支持。
2. WannaDance 团队仅对本程序提供合理的免费技术支持，包括：程序的使用、程序的功能、程序的问题、程序的更新。
3. WannaDance 团队保留对本条款的最终解释权。

[WannaDance]: https://vrchat.com/home/world/wrld_8ac0b9db-17ae-44af-9d20-7d8ab94a9129
[WannaDance Dev]: https://vrchat.com/home/world/wrld_b9aa3e07-330b-4eb3-8d71-7708c27e86d7
[VRChat]: https://vrchat.com/
  "#
  );
}

fn check_license_agreement() {
  let agree_file_exists = std::path::Path::new("I_AGREE_TO_THE_LICENSE.txt").exists();
  let agree_env_exists = std::env::var("I_AGREE_TO_THE_LICENSE")
    .map(|v| v == "YES")
    .unwrap_or(false);

  if !agree_env_exists && !agree_file_exists {
    println!("请在使用本程序之前阅读并同意使用条款，可以通过如下途径同意使用条款：");
    println!(
      "1. 在程序所在目录下创建文件 I_AGREE_TO_THE_LICENSE.txt 以同意使用条款，然后重新启动程序。"
    );
    println!("2. 如果你在不方便创建文件的环境下使用，请设置环境变量 I_AGREE_TO_THE_LICENSE 为 YES，然后重新启动程序。");
    println!("   环境变量可以通过以下方式设置：");
    println!("   1. 通过 Windows/Linux/macOs 系统设置环境变量");
    println!("   2. 通过程序目录下的 .env 文件设置环境变量");

    loop {
      std::thread::sleep(std::time::Duration::from_secs(1));
    }
  }
}

#[tokio::main]
async fn main() {
  match dotenvy::dotenv() {
    Err(e) => warn!("dotenv(): failed to load .env file: {}", e),
    _ => {}
  }

  print_license();
  check_license_agreement();

  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
    .filter(Some("warp::server"), log::LevelFilter::Off)
    .init();

  let opts = AppOpts::parse();

  info!(
    "WannaDance: starting daemon, version {}",
    wanna_cdn::my_git_hash()
  );
  info!("video path: {}", opts.video_path_ud);

  let app = AppServiceImpl::new(opts.clone())
    .await
    .expect("Failed to initialize app service");

  let http = tokio::spawn(wanna_cdn::http::serve_video_http(app.clone()));
  let rtsp = match opts.rtsp_listen.is_some() {
    true => tokio::spawn(wanna_cdn::rtsp::serve_rtsp_typewriter(app.clone())),
    false => {
      info!("RTSP disabled");
      tokio::task::spawn(async { Ok(()) })
    }
  };
  let (l4, l4_enabled) = match (&opts.builtin_sni_listen, &opts.builtin_sni_proxy) {
    (Some(listen), Some(proxy)) if !proxy.is_empty() && !listen.is_empty() => {
      let mut proxy_targets = HashMap::new();
      for target_def in proxy {
        // api.udon.dance=ud-orig.kiva.moe:443
        let mut parts = target_def.splitn(2, '=');
        if let (Some(host), Some(forward_target)) = (parts.next(), parts.next()) {
          proxy_targets.insert(host.to_string(), forward_target.to_string());
        }
      }
      (
        tokio::spawn(wanna_cdn::forward::serve_sni_proxy(
          listen.clone(),
          proxy_targets,
        )),
        true,
      )
    }
    _ => {
      info!("No SNI proxy configured");
      (tokio::task::spawn(async { Ok(()) }), false)
    }
  };

  tokio::select! {
      e = l4, if l4_enabled => {
          match e {
              Ok(Ok(_)) => info!("SNI proxy exited successfully"),
              Ok(Err(e)) => warn!("SNI proxy exited with error: {}", e),
              Err(e) => warn!("SNI proxy exited with error: {}", e),
          }
      },
      e = rtsp, if opts.rtsp_listen.is_some() => {
          match e {
              Ok(Ok(_)) => info!("RTSP exited successfully"),
              Ok(Err(e)) => warn!("RTSP exited with error: {}", e),
              Err(e) => warn!("RTSP exited with error: {}", e),
          }
      },
      e = http => {
          match e {
              Ok(Ok(_)) => info!("Server exited successfully"),
              Ok(Err(e)) => warn!("Server exited with error: {}", e),
              Err(e) => warn!("Server exited with error: {}", e),
          }
      },
      _ = tokio::signal::ctrl_c() => {
          info!("Received Ctrl-C, shutting down...");
      }
  }

  info!("Goodbye!");
}
