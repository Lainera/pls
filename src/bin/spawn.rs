use pls::controller;
use pls::runner::{job_request::*, JobRequest, LogMessage};

#[tokio::main]
async fn main() -> Result<(), pls::PlsError> {
    let mut args = std::env::args();

    let lim = std::env::var("LIMIT").is_ok();
    let name = args.nth(1).expect("Pass client name");
    let bin = args.next().expect("Pass bin");
    let args: Vec<String> = args.collect();

    println!("B({}), O({:?}), N({}), L({})", bin, args, name, lim);

    let req = if lim {
        JobRequest {
            executable: bin,
            cpu_control: Some(CpuControl { cpu_weight: 500 }),
            mem_control: Some(MemControl {
                mem_max: 1024 * 1024 * 1024,
                mem_high: 1024 * 1024 * 512,
            }),
            io_control: Some(IoControl {
                major: 8,
                minor: 1,
                rbps_max: 1024,
                wbps_max: 1024,
            }),
            args,
        }
    } else {
        JobRequest {
            executable: bin,
            cpu_control: None,
            mem_control: None,
            io_control: None,
            args,
        }
    };

    let mut ctr = controller::Controller::new(&name).await?;
    let first_id = ctr.start(req).await?;
    println!("Status: {:#?}", ctr);

    let mut output = ctr.output(first_id).await?;

    let f = tokio::spawn(async move {
        while let Some(Ok(msg)) = output.recv().await {
            eprintln!("F:: {}", Around(msg));
        }
    });

    f.await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::WouldBlock))?;

    Ok(())
}

struct Around(LogMessage);
impl core::fmt::Display for Around {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.fd == 0 {
            write!(f, "\u{001b}[32m")?;
        } else {
            write!(f, "\u{001b}[31m")?;
        }
        write!(
            f,
            "{:?}\u{001b}[0m",
            std::str::from_utf8(&self.0.output).unwrap_or("not-utf8")
        )
    }
}
