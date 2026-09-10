use futures::StreamExt;
use relm4::ComponentSender;
use wayle_config::ConfigService;

use crate::shell::bar::{Bar, BarCmd};

/// Spawns a task that reacts to `modules.plugins` changes.
///
/// This enables hot-reload for plugin definitions in config.toml,
/// even when the bar layout itself does not change.
pub(crate) fn spawn(sender: &ComponentSender<Bar>, config_service: &std::sync::Arc<ConfigService>) {
    let config = config_service.config().clone();
    let mut plugins_stream = config.modules.plugins.watch();

    sender.command(move |out, shutdown| async move {
        let mut initialized = false;
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);

        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                Some(_) = plugins_stream.next() => {
                    if !initialized {
                        initialized = true;
                        continue;
                    }
                    let _ = out.send(BarCmd::PluginDefinitionsChanged);
                }
            }
        }
    });
}
