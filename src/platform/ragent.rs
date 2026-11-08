//! Кластер, которого поднимает раннер: ключи запуска `ragent` и `ras`. Что запускать и в каком
//! порядке, решает `use_cases/managed_stand`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::platform::service::ServiceRequest;

/// Сколько портов в диапазоне рабочих процессов.
pub const WORKER_RANGE_WIDTH: u16 = 16;

/// Порты кластера. Закрепляются при первом запуске: реестр кластера (`reg_<regport>`) хранит
/// `regport`, порт агента и диапазон рабочих процессов (замер #538), а `regport` — ещё и адрес `Srvr=`.
/// Порт `ras` в реестр не попадает и выбирается заново при каждом запуске.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClusterPorts {
    pub agent: u16,
    pub regport: u16,
    pub range_start: u16,
}

impl ClusterPorts {
    /// Порты блоком от `first`. Первый порт выводится из пути копии, но блок шире шага между
    /// попытками: блок другой копии может задеть закреплённый, и тогда выход — `--new-regport`.
    pub fn from_first(first: u16) -> Self {
        Self {
            agent: first,
            regport: first + 1,
            range_start: first + 2,
        }
    }

    pub fn range_end(self) -> u16 {
        // Закреплённое читается из файла: правленное руками не должно переполнить порт.
        self.range_start.saturating_add(WORKER_RANGE_WIDTH - 1)
    }

    /// Все порты блока.
    pub fn all(self) -> impl Iterator<Item = u16> {
        [self.agent, self.regport]
            .into_iter()
            .chain(self.range_start..=self.range_end())
    }

    /// Строка соединения: клиент идёт на `regport` (замер #534).
    pub fn connection(self, name: &str) -> String {
        format!("Srvr=127.0.0.1:{};Ref={name}", self.regport)
    }

    /// Адрес агента для `ras`.
    pub fn agent_address(self) -> String {
        format!("127.0.0.1:{}", self.agent)
    }

    pub fn worker_range(self) -> String {
        format!("{}:{}", self.range_start, self.range_end())
    }
}

/// Адрес `ras` для `rac`.
pub fn ras_address(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// Запуск агента кластера с реестром в `registry` (каталог `/d`).
pub fn start_agent(
    ragent: &Path,
    registry: &Path,
    ports: ClusterPorts,
    log: &Path,
) -> ServiceRequest {
    ServiceRequest {
        program: ragent.to_path_buf(),
        args: vec![
            "/agent".to_owned(),
            "/port".to_owned(),
            ports.agent.to_string(),
            "/regport".to_owned(),
            ports.regport.to_string(),
            "/range".to_owned(),
            ports.worker_range(),
            "/d".to_owned(),
            registry.display().to_string(),
        ],
        workdir: registry.parent().map(Path::to_path_buf),
        log_path: log.to_path_buf(),
        stdin: None,
    }
}

/// Запуск `ras` к агенту. Без `--monitor-port`: `ras` 8.3.24 его не знает (замер #534).
pub fn start_ras(ras: &Path, port: u16, ports: ClusterPorts, log: &Path) -> ServiceRequest {
    ServiceRequest {
        program: ras.to_path_buf(),
        args: vec![
            "cluster".to_owned(),
            format!("--port={port}"),
            ports.agent_address(),
        ],
        workdir: log.parent().map(Path::to_path_buf),
        log_path: log.to_path_buf(),
        stdin: None,
    }
}

/// Ключ запуска, по которому узнают чужой `ragent` над тем же реестром.
pub fn registry_keys(registry: &Path) -> Vec<String> {
    vec![registry.display().to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ports_of_a_block_do_not_overlap_and_name_the_addresses() {
        let ports = ClusterPorts::from_first(24000);
        let mut all: Vec<u16> = ports.all().collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);
        assert!(all.iter().all(|port| (24000..24032).contains(port)));
        assert_eq!(ports.connection("demo"), "Srvr=127.0.0.1:24001;Ref=demo");
        assert_eq!(ports.worker_range(), "24002:24017");
    }

    #[test]
    fn ras_names_no_monitor_port() {
        let ports = ClusterPorts::from_first(24000);
        let request = start_ras(Path::new("ras.exe"), 26000, ports, Path::new("w/ras.log"));
        assert_eq!(request.args, ["cluster", "--port=26000", "127.0.0.1:24000"]);
        let agent = start_agent(
            Path::new("ragent.exe"),
            Path::new("w/data"),
            ports,
            Path::new("w/cluster.log"),
        );
        assert_eq!(
            agent.args,
            [
                "/agent",
                "/port",
                "24000",
                "/regport",
                "24001",
                "/range",
                "24002:24017",
                "/d",
                "w/data"
            ]
        );
    }
}
