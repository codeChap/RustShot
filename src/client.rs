use crate::cli::Command;
use crate::dbus::{OBJECT_PATH, SERVICE_NAME};
use anyhow::Result;
use zbus::Connection;

pub fn run(cmd: Command) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_async(cmd))
}

async fn run_async(cmd: Command) -> Result<()> {
    let conn = Connection::session().await?;
    let proxy = zbus::Proxy::new(&conn, SERVICE_NAME, OBJECT_PATH, SERVICE_NAME).await?;

    match cmd {
        Command::Gui(args) => {
            let path = path_string(args.path);
            proxy
                .call::<_, _, ()>(
                    "graphicCaptureFlags",
                    &(
                        path,
                        args.delay as u32,
                        args.clipboard,
                        args.no_save,
                        String::new(),
                    ),
                )
                .await?;
        }
        Command::Full(args) => {
            let path = path_string(args.path);
            proxy
                .call::<_, _, ()>(
                    "fullScreenFlags",
                    &(
                        path,
                        args.delay as u32,
                        args.clipboard,
                        args.no_save,
                        String::new(),
                    ),
                )
                .await?;
        }
        Command::Screen { common, number } => {
            let path = path_string(common.path);
            let n: i32 = number.map(|n| n as i32).unwrap_or(-1);
            proxy
                .call::<_, _, ()>(
                    "captureScreenFlags",
                    &(
                        n,
                        path,
                        common.delay as u32,
                        common.clipboard,
                        common.no_save,
                        String::new(),
                    ),
                )
                .await?;
        }
    }
    Ok(())
}

fn path_string(p: Option<std::path::PathBuf>) -> String {
    p.map(|p| p.display().to_string()).unwrap_or_default()
}
