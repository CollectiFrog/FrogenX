//! La distribution vers les sorties CPU — §8.0 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** La politique de perte y est écrite une
//! seule fois, et se teste avec des frames fabriquées à la main.

use crate::sink::{CpuFrame, CpuSink, PixelLayout, SinkStatus};
use crate::stream::{Cadence, StreamFormat};
use std::sync::atomic::{AtomicU32, Ordering};
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
    /// Frames perdues par l'étage, en plus de ce que la sortie rapporte.
    /// Atomique : le thread de relecture les compte aussi (§8.5).
    dropped: AtomicU32,
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

    fn count_lost(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Le thread de la sortie a disparu : elle a paniqué. La panique reste
    /// confinée à ce thread, la performance continue (§18.1).
    fn thread_lost(&self) {
        self.set(SinkStatus::Failed(
            "le thread de la sortie a paniqué".into(),
        ));
    }
}

/// Désigne une sortie CPU, pour la régler ou la retirer (§8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSinkId(u32);

/// Ce qu'une image doit être pour une sortie : deux sorties de même clé
/// partagent leur réduction, leur relecture et leurs octets (§8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameKey {
    pub layout: PixelLayout,
    pub width: u32,
    pub height: u32,
}

/// Le chemin vers la file d'une sortie : il traverse le thread de relecture.
#[derive(Clone)]
struct Route {
    sender: SyncSender<CpuFrame>,
    shared: Arc<Shared>,
}

impl Route {
    fn deliver(&self, frame: CpuFrame, policy: DropPolicy) {
        match policy {
            DropPolicy::Lossy => match self.sender.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => self.shared.count_lost(),
                Err(TrySendError::Disconnected(_)) => self.shared.thread_lost(),
            },
            DropPolicy::Lossless => {
                if self.sender.send(frame).is_err() {
                    self.shared.thread_lost();
                }
            }
        }
    }
}

/// Les sorties qui attendent la même image à cette frame.
#[derive(Clone)]
pub struct Group {
    pub key: FrameKey,
    policy: DropPolicy,
    routes: Vec<Route>,
}

impl Group {
    /// Dépose la frame dans chaque file, selon la politique de l'étage.
    pub fn deliver(&self, frame: CpuFrame) {
        for route in &self.routes {
            route.deliver(frame.clone(), self.policy);
        }
    }

    /// L'image n'a pas pu être produite : elle est perdue pour tout le groupe,
    /// et cela doit se voir.
    pub fn count_lost(&self) {
        for route in &self.routes {
            route.shared.count_lost();
        }
    }
}

/// Le format et la cadence d'un flux.
struct Stream {
    format: StreamFormat,
    cadence: Cadence,
}

impl Stream {
    fn new(format: StreamFormat) -> Self {
        Self {
            format,
            cadence: Cadence::new(format.fps),
        }
    }

    fn key(&self, render: (u32, u32)) -> FrameKey {
        let (width, height) = self.format.within(render);
        FrameKey {
            layout: PixelLayout::BgraOpaque,
            width,
            height,
        }
    }
}

struct Channel {
    id: CpuSinkId,
    /// `None` une fois la file fermée par [`CpuFanout::finish`].
    sender: Option<SyncSender<CpuFrame>>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// `None` : la sortie reçoit chaque frame, à la taille du rendu.
    stream: Option<Stream>,
}

impl Channel {
    fn status(&self) -> SinkStatus {
        let perdues = self.shared.dropped.load(Ordering::Relaxed);
        match self.shared.get() {
            SinkStatus::Active if perdues > 0 => SinkStatus::Degraded { dropped: perdues },
            SinkStatus::Degraded { dropped } => SinkStatus::Degraded {
                dropped: dropped.saturating_add(perdues),
            },
            autre => autre,
        }
    }

    fn close(&mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                self.shared.thread_lost();
            }
        }
    }
}

/// Une file bornée et un thread par sortie CPU.
pub struct CpuFanout {
    policy: DropPolicy,
    channels: Vec<Channel>,
    next_id: u32,
}

impl CpuFanout {
    pub fn new(policy: DropPolicy) -> Self {
        Self {
            policy,
            channels: Vec::new(),
            next_id: 0,
        }
    }

    pub fn policy(&self) -> DropPolicy {
        self.policy
    }

    /// Lance le thread de la sortie. Une sortie qui passe en `Failed` n'est
    /// plus appelée, même pour les frames déjà en file.
    pub fn add(&mut self, sink: Box<dyn CpuSink>) -> CpuSinkId {
        self.spawn(sink, None)
    }

    /// Comme [`Self::add`], pour une sortie qui a son format et sa cadence.
    pub fn add_stream(&mut self, sink: Box<dyn CpuSink>, format: StreamFormat) -> CpuSinkId {
        self.spawn(sink, Some(Stream::new(format)))
    }

    fn spawn(&mut self, mut sink: Box<dyn CpuSink>, stream: Option<Stream>) -> CpuSinkId {
        let (sender, receiver) = mpsc::sync_channel::<CpuFrame>(QUEUE_DEPTH);
        let shared = Arc::new(Shared {
            status: Mutex::new(sink.status()),
            dropped: AtomicU32::new(0),
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
        let id = CpuSinkId(self.next_id);
        self.next_id += 1;
        self.channels.push(Channel {
            id,
            sender: Some(sender),
            shared,
            thread: Some(thread),
            stream,
        });
        id
    }

    /// Change le format d'un flux, entre deux frames. Rend `false` si ce n'est
    /// pas un flux, ou s'il est retiré.
    pub fn set_stream_format(&mut self, id: CpuSinkId, format: StreamFormat) -> bool {
        let flux = self.channels.iter_mut().find(|c| c.id == id);
        let Some(flux) = flux.and_then(|c| c.stream.as_mut()) else {
            return false;
        };
        if flux.format.fps != format.fps {
            flux.cadence = Cadence::new(format.fps);
        }
        flux.format = format;
        true
    }

    /// Retire une sortie et attend la fin de son thread : le sender Spout doit
    /// avoir libéré son nom au retour (§8.5). Rend `false` si elle est déjà
    /// retirée.
    pub fn remove(&mut self, id: CpuSinkId) -> bool {
        let Some(i) = self.channels.iter().position(|c| c.id == id) else {
            return false;
        };
        self.channels.remove(i).close();
        true
    }

    /// Le statut d'une sortie.
    pub fn status(&self, id: CpuSinkId) -> Option<SinkStatus> {
        let channel = self.channels.iter().find(|c| c.id == id)?;
        Some(channel.status())
    }

    /// Les images que les sorties attendent à cette frame, chacune une fois.
    /// Une frame que personne n'attend ne coûte rien : ni réduction, ni
    /// relecture. Fait avancer la cadence des flux (§8.5).
    pub fn due(&mut self, t: f64, render: (u32, u32)) -> Vec<Group> {
        let policy = self.policy;
        let mut groupes: Vec<Group> = Vec::new();
        for channel in &mut self.channels {
            let Some(sender) = &channel.sender else {
                continue;
            };
            if matches!(channel.status(), SinkStatus::Failed(_)) {
                continue;
            }
            let key = match &mut channel.stream {
                Some(flux) => {
                    if !flux.cadence.due(t) {
                        continue;
                    }
                    flux.key(render)
                }
                None => FrameKey {
                    layout: PixelLayout::RgbaStraight,
                    width: render.0,
                    height: render.1,
                },
            };
            let route = Route {
                sender: sender.clone(),
                shared: Arc::clone(&channel.shared),
            };
            match groupes.iter_mut().find(|g| g.key == key) {
                Some(groupe) => groupe.routes.push(route),
                None => groupes.push(Group {
                    key,
                    policy,
                    routes: vec![route],
                }),
            }
        }
        groupes
    }

    /// Les clés que les sorties peuvent demander sous ce rendu : le reste des
    /// cibles de l'étage peut être libéré.
    pub fn keys(&self, render: (u32, u32)) -> Vec<FrameKey> {
        self.channels
            .iter()
            .map(|c| match &c.stream {
                Some(flux) => flux.key(render),
                None => FrameKey {
                    layout: PixelLayout::RgbaStraight,
                    width: render.0,
                    height: render.1,
                },
            })
            .collect()
    }

    /// Dépose une frame déjà produite dans la file de chaque sortie qui
    /// l'attend, selon la politique de l'étage.
    pub fn publish(&mut self, frame: CpuFrame) {
        for groupe in self.due(frame.t, (frame.width, frame.height)) {
            groupe.deliver(frame.clone());
        }
    }

    /// Une frame n'a pas pu être produite (relecture échouée) : elle est
    /// perdue pour toutes les sorties, et cela doit se voir.
    pub fn count_lost(&mut self) {
        for channel in &self.channels {
            channel.shared.count_lost();
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
            channel.close();
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
            layout: PixelLayout::RgbaStraight,
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
        assert!(fanout.due(0.0, (1, 1)).is_empty());
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
        assert!(CpuFanout::new(DropPolicy::Lossless)
            .due(0.0, (1, 1))
            .is_empty());
    }

    const RENDU: (u32, u32) = (1920, 1080);

    fn flux(width: u32, height: u32, fps: u32) -> StreamFormat {
        StreamFormat { width, height, fps }
    }

    fn cle(width: u32, height: u32) -> FrameKey {
        FrameKey {
            layout: PixelLayout::BgraOpaque,
            width,
            height,
        }
    }

    fn cles(groupes: &[Group]) -> Vec<(FrameKey, usize)> {
        groupes.iter().map(|g| (g.key, g.routes.len())).collect()
    }

    #[test]
    fn deux_flux_de_meme_format_partagent_leur_image() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 60));
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 60));
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(640, 360, 60));
        assert_eq!(
            cles(&fanout.due(0.0, RENDU)),
            vec![(cle(1280, 720), 2), (cle(640, 360), 1)]
        );
    }

    #[test]
    fn une_sortie_sans_format_recoit_le_rendu_en_alpha_droit() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        fanout.add(Box::new(Fake::new(&recu)));
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(1920, 1080, 60));
        let groupes = fanout.due(0.0, RENDU);
        assert_eq!(groupes.len(), 2, "alpha droit et image opaque diffèrent");
        assert_eq!(groupes[0].key.layout, PixelLayout::RgbaStraight);
        assert_eq!(groupes[1].key.layout, PixelLayout::BgraOpaque);
    }

    #[test]
    fn une_frame_que_personne_n_attend_ne_coute_rien() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 30));
        assert_eq!(fanout.due(0.0, RENDU).len(), 1);
        assert!(fanout.due(1.0 / 60.0, RENDU).is_empty());
        assert_eq!(fanout.due(2.0 / 60.0, RENDU).len(), 1);
    }

    #[test]
    fn un_flux_est_borne_par_le_rendu() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(3840, 2160, 60));
        assert_eq!(cles(&fanout.due(0.0, RENDU)), vec![(cle(1920, 1080), 1)]);
    }

    #[test]
    fn changer_de_format_prend_effet_a_la_frame_suivante() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        let id = fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 60));
        assert!(fanout.set_stream_format(id, flux(640, 360, 60)));
        assert_eq!(cles(&fanout.due(0.0, RENDU)), vec![(cle(640, 360), 1)]);
    }

    #[test]
    fn changer_de_taille_ne_decale_pas_la_cadence() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        let id = fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 30));
        assert_eq!(fanout.due(0.0, RENDU).len(), 1);
        assert!(fanout.set_stream_format(id, flux(640, 360, 30)));
        assert!(fanout.due(1.0 / 60.0, RENDU).is_empty());
    }

    #[test]
    fn une_sortie_sans_format_ne_se_regle_pas() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        let id = fanout.add(Box::new(Fake::new(&recu)));
        assert!(!fanout.set_stream_format(id, flux(640, 360, 30)));
    }

    #[test]
    fn retirer_un_flux_finit_sa_file_et_ne_sert_qu_une_fois() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        let id = fanout.add(Box::new(Fake::new(&recu)));
        fanout.publish(frame(0));
        assert!(fanout.remove(id));
        assert_eq!(*recu.lock().unwrap(), vec![0], "la file se vide d'abord");
        assert!(!fanout.remove(id));
        assert_eq!(fanout.status(id), None);
        assert!(!fanout.set_stream_format(id, flux(640, 360, 30)));
    }

    #[test]
    fn un_identifiant_retire_ne_resert_jamais() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossless);
        let ancien = fanout.add(Box::new(Fake::new(&recu)));
        fanout.remove(ancien);
        let nouveau = fanout.add(Box::new(Fake::new(&recu)));
        assert_ne!(ancien, nouveau);
    }

    #[test]
    fn une_frame_perdue_pour_un_groupe_ne_touche_que_lui() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        let a = fanout.add_stream(Box::new(Fake::new(&recu)), flux(1280, 720, 60));
        let b = fanout.add_stream(Box::new(Fake::new(&recu)), flux(640, 360, 60));
        fanout.due(0.0, RENDU)[0].count_lost();
        assert_eq!(fanout.status(a), Some(SinkStatus::Degraded { dropped: 1 }));
        assert_eq!(fanout.status(b), Some(SinkStatus::Active));
    }

    #[test]
    fn les_cles_disent_quelles_cibles_restent_utiles() {
        let recu = recus();
        let mut fanout = CpuFanout::new(DropPolicy::Lossy);
        fanout.add_stream(Box::new(Fake::new(&recu)), flux(3840, 2160, 30));
        assert_eq!(fanout.keys(RENDU), vec![cle(1920, 1080)]);
        assert_eq!(fanout.keys((1280, 720)), vec![cle(1280, 720)]);
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
        assert!(fanout.due(0.0, (1, 1)).is_empty());
    }
}
