//! La relecture en direct — §8.2, §8.5 de `docs/ARCHITECTURE.md`.
//!
//! La boucle copie l'image dans un buffer et repart. Un thread attend le GPU,
//! recopie les octets et les remet aux files : le retard suit le travail du
//! GPU, pas la cadence du moteur.

use crate::fanout::Group;
use crate::sink::CpuFrame;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Les buffers de relecture d'un format : deux frames en vol, plus celle qu'on
/// recopie. Au-delà, la frame est perdue et comptée, la boucle ne ralentit pas.
pub const READBACK_SLOTS: usize = 3;
/// Au-delà, le GPU ne répond plus : la frame est perdue, jamais la performance.
const GPU_TIMEOUT: Duration = Duration::from_secs(1);

/// Les buffers libres d'un format, partagés avec le thread qui les rend.
pub(crate) type Pool = Arc<Mutex<Vec<wgpu::Buffer>>>;

pub(crate) fn pool_take(pool: &Pool) -> Option<wgpu::Buffer> {
    pool.lock().unwrap_or_else(PoisonError::into_inner).pop()
}

fn pool_give(pool: &Pool, buffer: wgpu::Buffer) {
    pool.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(buffer);
}

/// Une image à recopier : `frame.pixels` reste vide jusque-là.
pub(crate) struct Readback {
    pub buffer: wgpu::Buffer,
    pub pool: Pool,
    pub group: Group,
    pub frame: CpuFrame,
    pub submission: wgpu::SubmissionIndex,
    /// La ligne utile et sa place dans le buffer, en octets.
    pub row: usize,
    pub stride: usize,
}

enum Task {
    Read(Box<Readback>),
    /// Réveille son émetteur une fois tout ce qui précède terminé.
    Flush(Sender<()>),
}

/// Le thread qui attend le GPU.
pub(crate) struct LiveReader {
    tasks: Option<Sender<Task>>,
    thread: Option<JoinHandle<()>>,
}

impl LiveReader {
    pub fn new(device: wgpu::Device) -> Self {
        let (tasks, recues) = mpsc::channel::<Task>();
        let thread = std::thread::spawn(move || {
            while let Ok(task) = recues.recv() {
                match task {
                    Task::Read(lecture) => (*lecture).run(&device),
                    Task::Flush(fini) => {
                        let _ = fini.send(());
                    }
                }
            }
        });
        Self {
            tasks: Some(tasks),
            thread: Some(thread),
        }
    }

    pub fn read(&self, lecture: Readback) {
        let Some(tasks) = &self.tasks else { return };
        if let Err(mpsc::SendError(Task::Read(lecture))) = tasks.send(Task::Read(Box::new(lecture)))
        {
            lecture.group.count_lost();
        }
    }

    /// Attend la fin des relectures en cours. Sert en édition : retirer un flux,
    /// finir un rendu.
    pub fn flush(&self) {
        let Some(tasks) = &self.tasks else { return };
        let (fini, attente) = mpsc::channel();
        if tasks.send(Task::Flush(fini)).is_ok() {
            let _ = attente.recv();
        }
    }
}

impl Drop for LiveReader {
    fn drop(&mut self) {
        self.tasks = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Readback {
    fn run(self, device: &wgpu::Device) {
        let lecture = self;
        let (pret, attente) = mpsc::channel();
        let tranche = lecture.buffer.slice(..);
        tranche.map_async(wgpu::MapMode::Read, move |r| {
            let _ = pret.send(r);
        });
        let attendu = device.poll(wgpu::PollType::Wait {
            submission_index: Some(lecture.submission),
            timeout: Some(GPU_TIMEOUT),
        });
        // Un autre `poll` a pu servir le rappel : seul son résultat compte.
        let carte = match attente.recv_timeout(GPU_TIMEOUT) {
            Ok(Ok(())) if attendu.is_ok() => tranche.get_mapped_range().ok(),
            Ok(_) | Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        };
        let pixels = carte
            .as_ref()
            .map(|octets| tight_rows(octets, lecture.row, lecture.stride));
        drop(carte);
        lecture.buffer.unmap();
        pool_give(&lecture.pool, lecture.buffer);
        match pixels {
            Some(pixels) => lecture.group.deliver(CpuFrame {
                pixels,
                ..lecture.frame
            }),
            None => lecture.group.count_lost(),
        }
    }
}

/// Retire le rembourrage des lignes : wgpu les aligne sur 256 octets. Une ligne
/// déjà alignée se recopie d'un bloc.
fn tight_rows(octets: &[u8], row: usize, stride: usize) -> Arc<[u8]> {
    if row == stride {
        return Arc::from(octets);
    }
    let mut serre = Vec::with_capacity(octets.len() / stride * row);
    for ligne in octets.chunks_exact(stride) {
        serre.extend_from_slice(&ligne[..row]);
    }
    Arc::from(serre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_ligne_alignee_passe_telle_quelle() {
        assert_eq!(&*tight_rows(&[1, 2, 3, 4], 4, 4), &[1, 2, 3, 4]);
    }

    #[test]
    fn le_rembourrage_disparait() {
        let octets = [1, 2, 0, 0, 3, 4, 0, 0];
        assert_eq!(&*tight_rows(&octets, 2, 4), &[1, 2, 3, 4]);
    }
}
