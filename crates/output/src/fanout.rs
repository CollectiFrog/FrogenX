//! La distribution vers les sorties CPU — §8.0 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** La politique de perte y est écrite une
//! seule fois, et se teste avec des frames fabriquées à la main.

use crate::sink::{CpuFrame, CpuSink, SinkStatus};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// Profondeur de la file de chaque sortie, en frames (§8.0).
pub const QUEUE_DEPTH: usize = 3;

/// Ce que fait l'étage quand une sortie n'arrive pas à suivre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPolicy {
    /// `Clock::Realtime` : la frame est perdue et comptée, la boucle ne
    /// ralentit jamais.
    Lossy,
    /// `Clock::Fixed` : la boucle attend la sortie la plus lente, aucune frame
    /// n'est perdue.
    Lossless,
}

/// État partagé entre l'étage et le thread de la sortie.
struct Shared {
    /// Dernier statut rapporté par la sortie.
    status: Mutex<SinkStatus>,
}

impl Shared {
    fn get(&self) -> SinkStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, status: SinkStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status;
    }
}

struct Channel {
    /// `None` une fois la file fermée par [`CpuFanout::finish`].
    sender: Option<SyncSender<CpuFrame>>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// Frames perdues par l'étage, en plus de ce que la sortie rapporte.
    dropped: u32,
}

impl Channel {
    fn status(&self) -> SinkStatus {
        match self.shared.get() {
            SinkStatus::Active if self.dropped > 0 => SinkStatus::Degraded {
                dropped: self.dropped,
            },
            SinkStatus::Degraded { dropped } => SinkStatus::Degraded {
                dropped: dropped.saturating_add(self.dropped),
            },
            autre => autre,
        }
    }

    fn deposit(&mut self, frame: CpuFrame, policy: DropPolicy) {
        let Some(sender) = &self.sender else { return };
        let perdue = match policy {
            DropPolicy::Lossy => match sender.try_send(frame) {
                Ok(()) => false,
                Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => {
                    self.thread_lost();
                    false
                }
            },
            DropPolicy::Lossless => {
                if sender.send(frame).is_err() {
                    self.thread_lost();
                }
                false
            }
        };
        if perdue {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Le thread de la sortie a disparu : elle a paniqué. La panique reste
    /// confinée à ce thread, la performance continue (§18.1).
    fn thread_lost(&mut self) {
        self.shared.set(SinkStatus::Failed(
            "le thread de la sortie a paniqué".into(),
        ));
    }
}

/// Une file bornée et un thread par sortie CPU.
pub struct CpuFanout {
    policy: DropPolicy,
    channels: Vec<Channel>,
}

impl CpuFanout {
    pub fn new(policy: DropPolicy) -> Self {
        Self {
            policy,
            channels: Vec::new(),
        }
    }

    pub fn policy(&self) -> DropPolicy {
        self.policy
    }

    /// Lance le thread de la sortie. Une sortie qui passe en `Failed` n'est
    /// plus appelée, même pour les frames déjà en file.
    pub fn add(&mut self, mut sink: Box<dyn CpuSink>) {
        let (sender, receiver) = mpsc::sync_channel::<CpuFrame>(QUEUE_DEPTH);
        let shared = Arc::new(Shared {
            status: Mutex::new(sink.status()),
        });
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::spawn(move || {
            while let Ok(frame) = receiver.recv() {
                if matches!(sink.status(), SinkStatus::Failed(_)) {
                    continue;
                }
                sink.submit(&frame);
                thread_shared.set(sink.status());
            }
        });
        self.channels.push(Channel {
            sender: Some(sender),
            shared,
            thread: Some(thread),
            dropped: 0,
        });
    }

    /// Une sortie au moins attend des frames : sinon, la relecture, la
    /// plus chère des opérations du pipeline, n'a pas lieu d'être.
    pub fn has_active(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.sender.is_some() && !matches!(c.status(), SinkStatus::Failed(_)))
    }

    /// Dépose une frame dans la file de chaque sortie active, selon la
    /// politique de l'étage.
    pub fn publish(&mut self, frame: CpuFrame) {
        let policy = self.policy;
        for channel in &mut self.channels {
            if !matches!(channel.status(), SinkStatus::Failed(_)) {
                channel.deposit(frame.clone(), policy);
            }
        }
    }

    /// Une frame n'a pas pu être produite (relecture échouée) : elle est
    /// perdue pour toutes les sorties, et cela doit se voir.
    pub fn count_lost(&mut self) {
        for channel in &mut self.channels {
            channel.dropped = channel.dropped.saturating_add(1);
        }
    }

    /// Le statut de chaque sortie, dans l'ordre d'ajout. Une sortie `Failed`
    /// reste listée.
    pub fn statuses(&self) -> Vec<SinkStatus> {
        self.channels.iter().map(Channel::status).collect()
    }

    /// Ferme les files et attend que chaque sortie ait fini : à la fin d'un
    /// rendu offline, toutes les frames sont écrites. Idempotent.
    pub fn finish(&mut self) {
        for channel in &mut self.channels {
            channel.sender = None;
            if let Some(thread) = channel.thread.take() {
                if thread.join().is_err() {
                    channel.thread_lost();
                }
            }
        }
    }
}

impl Drop for CpuFanout {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{sync_channel, Receiver};
    use std::time::Duration;

    fn frame(index: u64) -> CpuFrame {
        CpuFrame {
            pixels: Arc::from(vec![0u8; 4]),
            width: 1,
            height: 1,
            index,
            t: index as f64 / 60.0,
        }
    }

    /// Note les indices reçus ; `gate`, si elle est fournie, retient chaque
    /// frame jusqu'à recevoir un jeton — une sortie bloquée à la demande.
    struct Fake {
        received: Arc<Mutex<Vec<u64>>>,
        gate: Option<Receiver<()>>,
        delay: Duration,
        fail_at: Option<u64>,
        panic_at: Option<u64>,
        failed: bool,
    }

    impl Fake {
        fn new(received: &Arc<Mutex<Vec<u64>>>) -> Self {
            Self {
                received: Arc::clone(received),
                gate: None,
                delay: Duration::ZERO,
                fail_at: None,
                panic_at: None,
                failed: false,
            }
        }
    }

    impl CpuSink for Fake {
        fn submit(&mut self, frame: &CpuFrame) {
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            std::thread::sleep(self.delay);
            assert_ne!(self.panic_at, Some(frame.index), "panique voulue");
            self.received.lock().unwrap().push(frame.index);
            if self.fail_at == Some(frame.index) {
                self.failed = true;
            }
        }

        fn status(&self) -> SinkStatus {
            if self.failed {
                SinkStatus::Failed("disque plein".into())
            } else {
                SinkStatus::Active
            }
        }
    }

    fn recus() -> Arc<Mutex<Vec<u64>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[test]
    fn sans_perte_une_sortie_lente_recoit_tout_dans_l_ordre() {
        let recu = recus();
        let mut lente = Fake::new(&recu);
        lente.delay = Duration::from_millis(1);
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(lente));

        for i in 0..40 {
            fanout.publish(frame(i));
        }
        fanout.finish();

        let attendu: Vec<u64> = (0..40).collect();
        assert_eq!(*recu.lock().unwrap(), attendu);
        assert_eq!(fanout.statuses(), vec![SinkStatus::Active]);
    }

    #[test]
    fn avec_perte_une_sortie_bloquee_ne_retient_pas_la_boucle() {
        let recu = recus();
        let (jeton, gate) = sync_channel::<()>(64);
        let mut bloquee = Fake::new(&recu);
        bloquee.gate = Some(gate);
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        fanout.add(Box::new(bloquee));

        // Aucun jeton pendant ces dépôts : si `publish` attendait la sortie,
        // ce test ne se terminerait jamais.
        const N: u32 = 20;
        for i in 0..N {
            fanout.publish(frame(i as u64));
        }
        let SinkStatus::Degraded { dropped } = fanout.statuses()[0] else {
            panic!("attendu Degraded, got {:?}", fanout.statuses());
        };
        // La file tient QUEUE_DEPTH frames, plus celle que le thread a peut-être
        // déjà prise.
        let acceptees = N - dropped;
        assert!(
            (QUEUE_DEPTH as u32..=QUEUE_DEPTH as u32 + 1).contains(&acceptees),
            "{acceptees} frames acceptées"
        );

        for _ in 0..N {
            jeton.send(()).unwrap();
        }
        fanout.finish();

        let recu = recu.lock().unwrap();
        assert_eq!(recu.len() as u32, acceptees, "perdues + reçues == N");
        assert!(recu.windows(2).all(|p| p[0] < p[1]), "ordre : {recu:?}");
    }

    #[test]
    fn une_sortie_en_echec_n_est_plus_appelee_mais_reste_listee() {
        let recu = recus();
        let mut fragile = Fake::new(&recu);
        fragile.fail_at = Some(2);
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(fragile));

        for i in 0..10 {
            fanout.publish(frame(i));
        }
        fanout.finish();

        assert_eq!(*recu.lock().unwrap(), vec![0, 1, 2]);
        assert_eq!(
            fanout.statuses(),
            vec![SinkStatus::Failed("disque plein".into())]
        );
        assert!(!fanout.has_active());
    }

    #[test]
    fn une_sortie_en_echec_ne_prive_pas_les_autres() {
        let (a, b) = (recus(), recus());
        let mut fragile = Fake::new(&a);
        fragile.fail_at = Some(0);
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(fragile));
        fanout.add(Box::new(Fake::new(&b)));

        for i in 0..8 {
            fanout.publish(frame(i));
        }
        fanout.finish();

        assert_eq!(*a.lock().unwrap(), vec![0]);
        assert_eq!(*b.lock().unwrap(), (0..8).collect::<Vec<u64>>());
        assert_eq!(fanout.statuses()[1], SinkStatus::Active);
    }

    #[test]
    fn une_sortie_qui_panique_passe_en_echec_sans_propager() {
        let recu = recus();
        let mut instable = Fake::new(&recu);
        instable.panic_at = Some(1);
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(instable));

        for i in 0..8 {
            fanout.publish(frame(i));
        }
        fanout.finish();

        assert_eq!(*recu.lock().unwrap(), vec![0]);
        assert!(
            matches!(fanout.statuses()[0], SinkStatus::Failed(_)),
            "got {:?}",
            fanout.statuses()
        );
    }

    #[test]
    fn une_relecture_perdue_se_voit_sur_chaque_sortie() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(Fake::new(&recu)));
        fanout.count_lost();
        assert_eq!(fanout.statuses(), vec![SinkStatus::Degraded { dropped: 1 }]);
    }

    #[test]
    fn sans_sortie_rien_n_est_actif() {
        assert!(!CpuFanout::new(DropPolicy::Lossless).has_active());
    }

    #[test]
    fn finish_est_idempotent_et_coupe_les_frames_suivantes() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(Fake::new(&recu)));
        fanout.publish(frame(0));
        fanout.finish();
        fanout.finish();
        fanout.publish(frame(1));
        assert_eq!(*recu.lock().unwrap(), vec![0]);
        assert!(!fanout.has_active());
    }
}
