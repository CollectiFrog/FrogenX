//! Le sender Spout — §8.5 de `docs/ARCHITECTURE.md`.
//!
//! Repris de `bench/kybench/src/spout.rs` de KyberFrog (© Tristan Perrault,
//! AGPL-3.0-or-later), adapté au contrat de [`CpuSink`]. Le protocole :
//!
//! - `SpoutSenderNames` : un tableau de noms de 256 octets, le registre ;
//! - `ActiveSenderName` : le sender par défaut ;
//! - `<nom>` : 280 octets, poignée de partage, taille, format DXGI ;
//! - `<nom>SpoutAccessMutex` : protège l'accès à la texture ;
//! - `<nom>_Count_Semaphore` : le compteur de frames, un cran par frame publiée.
//!
//! **Le seul `unsafe` du projet avec celui de `wgpu`** : Direct3D 11 et les
//! objets nommés de Windows (§8.1, §10). Il reste dans ce fichier.

use crate::sink::{CpuFrame, CpuSink, PixelLayout, SinkStatus};
use std::ffi::c_void;
use std::fmt;
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HMODULE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Asynchronous, ID3D11Device, ID3D11DeviceContext, ID3D11Query,
    ID3D11Resource, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_QUERY_DESC, D3D11_QUERY_EVENT,
    D3D11_RESOURCE_MISC_SHARED, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIResource,
};
use windows::Win32::System::Memory::{
    CreateFileMappingA, MapViewOfFile, OpenFileMappingA, UnmapViewOfFile, VirtualQuery, FILE_MAP,
    FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateMutexA, CreateSemaphoreA, ReleaseMutex, ReleaseSemaphore, WaitForSingleObject,
};

const NAME_LEN: usize = 256;
const MAX_SENDERS: usize = 256;
const INFO_LEN: usize = 280;
/// L'attente du SDK Spout sur le mutex d'un sender. Au-delà, le récepteur tient
/// l'image : la frame est perdue, pas la cadence.
const MUTEX_TIMEOUT_MS: u32 = 67;
/// `DXGI_FORMAT_B8G8R8A8_UNORM`, le format commun à tous les récepteurs.
const DXGI_BGRA: u32 = 87;
const BYTES_PER_PIXEL: u32 = 4;

/// L'ouverture d'un sender a échoué — régime d'édition (§18.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpoutError {
    /// Le nom est vide ou trop long pour le registre.
    InvalidName,
    /// Un sender vivant porte déjà ce nom.
    NameTaken(String),
    /// Les 256 places du registre sont prises.
    RegistryFull,
    /// Direct3D 11 ou un objet de Windows a refusé.
    System(String),
}

impl fmt::Display for SpoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => write!(f, "nom de flux vide ou trop long"),
            Self::NameTaken(nom) => write!(f, "un flux Spout s'appelle déjà « {nom} »"),
            Self::RegistryFull => write!(f, "le registre Spout est plein"),
            Self::System(e) => write!(f, "Spout indisponible : {e}"),
        }
    }
}

impl std::error::Error for SpoutError {}

impl From<windows::core::Error> for SpoutError {
    fn from(e: windows::core::Error) -> Self {
        Self::System(e.message())
    }
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// Le device D3D11 sur l'adaptateur de wgpu : sur un autre, le récepteur ne
/// pourrait pas ouvrir la texture. Faute de le retrouver, le premier adaptateur.
fn device_on(vendor: u32, device: u32) -> Result<(ID3D11Device, ID3D11DeviceContext), SpoutError> {
    let adaptateur = find_adapter(vendor, device)?;
    let (mut d3d, mut contexte) = (None, None);
    // SAFETY : les sorties sont des `Option` locales, valides pendant l'appel.
    unsafe {
        D3D11CreateDevice(
            &adaptateur,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            Some(&mut contexte),
        )?;
    }
    match (d3d, contexte) {
        (Some(d3d), Some(contexte)) => Ok((d3d, contexte)),
        _ => Err(SpoutError::System("pas de device Direct3D 11".into())),
    }
}

fn find_adapter(vendor: u32, device: u32) -> Result<IDXGIAdapter1, SpoutError> {
    // SAFETY : énumération COM, sans pointeur conservé.
    unsafe {
        let usine: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut premier = None;
        let mut i = 0;
        while let Ok(adaptateur) = usine.EnumAdapters1(i) {
            let desc = adaptateur.GetDesc1()?;
            if desc.VendorId == vendor && desc.DeviceId == device {
                return Ok(adaptateur);
            }
            premier.get_or_insert(adaptateur);
            i += 1;
        }
        premier.ok_or_else(|| SpoutError::System("aucun adaptateur DXGI".into()))
    }
}

fn shared_texture(d3d: &ID3D11Device, w: u32, h: u32) -> Result<ID3D11Texture2D, SpoutError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
    };
    let mut texture = None;
    // SAFETY : `desc` et la sortie vivent le temps de l'appel.
    unsafe { d3d.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.ok_or_else(|| SpoutError::System("pas de texture partagée".into()))
}

/// Une vue sur un bloc de mémoire nommé.
struct Mapping {
    handle: HANDLE,
    view: *mut u8,
}

impl Mapping {
    fn create(name: &str, size: usize) -> Result<Self, SpoutError> {
        let n = cstr(name);
        // SAFETY : `n` finit par un zéro et vit le temps de l'appel.
        unsafe {
            let handle = CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                PCSTR(n.as_ptr()),
            )?;
            Self::map(handle, FILE_MAP_ALL_ACCESS.0, size)
        }
    }

    fn open(name: &str, access: u32, size: usize) -> Result<Self, SpoutError> {
        let n = cstr(name);
        // SAFETY : `n` finit par un zéro et vit le temps de l'appel.
        unsafe {
            let handle = OpenFileMappingA(access, false, PCSTR(n.as_ptr()))?;
            Self::map(handle, access, size)
        }
    }

    /// Le registre, entier : sa taille est celle qu'a choisie son créateur
    /// (TouchDesigner : 112 places), et une vue plus grande serait refusée.
    fn open_registry(access: u32) -> Result<(Self, usize), SpoutError> {
        let carte = Self::open("SpoutSenderNames", access, 0)?;
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY : `carte.view` est une vue vivante, `info` une sortie locale.
        let lu = unsafe {
            VirtualQuery(
                Some(carte.view as *const c_void),
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let places = if lu == 0 {
            0
        } else {
            (info.RegionSize / NAME_LEN).min(MAX_SENDERS)
        };
        Ok((carte, places))
    }

    /// # Safety
    /// `handle` est un bloc nommé valide, dont ce `Mapping` prend possession.
    unsafe fn map(handle: HANDLE, access: u32, size: usize) -> Result<Self, SpoutError> {
        // SAFETY : garanti par l'appelant.
        let vue = unsafe { MapViewOfFile(handle, FILE_MAP(access), 0, 0, size) };
        if vue.Value.is_null() {
            let e = windows::core::Error::from_thread();
            // SAFETY : la poignée reste à nous, la vue ayant échoué.
            let _ = unsafe { CloseHandle(handle) };
            return Err(e.into());
        }
        Ok(Self {
            handle,
            view: vue.Value as *mut u8,
        })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY : la vue et la poignée sont à nous et ne servent plus.
        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view as *mut c_void,
            });
            let _ = CloseHandle(self.handle);
        }
    }
}

/// # Safety
/// `src` désigne `NAME_LEN` octets lisibles.
unsafe fn read_name(src: *const u8) -> String {
    // SAFETY : garanti par l'appelant.
    let octets = unsafe { std::slice::from_raw_parts(src, NAME_LEN) };
    let n = octets.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    String::from_utf8_lossy(&octets[..n]).into_owned()
}

/// # Safety
/// `dst` désigne `NAME_LEN` octets inscriptibles.
unsafe fn write_name(dst: *mut u8, name: &str) {
    // SAFETY : garanti par l'appelant ; le nom est tronqué à la place disponible.
    unsafe {
        std::ptr::write_bytes(dst, 0, NAME_LEN);
        std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len().min(NAME_LEN - 1));
    }
}

/// La place du nom dans le registre : la sienne s'il y est déjà sans sender
/// vivant (un sender tué ne se désinscrit pas), sinon la première libre.
fn claim_slot(places: &[String], name: &str, vivant: bool) -> Result<usize, SpoutError> {
    match places.iter().position(|s| s == name) {
        Some(_) if vivant => Err(SpoutError::NameTaken(name.to_string())),
        Some(orpheline) => Ok(orpheline),
        None => places
            .iter()
            .position(|s| s.is_empty())
            .ok_or(SpoutError::RegistryFull),
    }
}

/// Une sortie vers Spout. Un `CpuSink` : son thread attend le GPU D3D11.
pub struct SpoutSink {
    name: String,
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    texture: ID3D11Texture2D,
    query: ID3D11Query,
    size: (u32, u32),
    names: Mapping,
    slots: usize,
    _active: Mapping,
    info: Mapping,
    mutex: HANDLE,
    semaphore: HANDLE,
    dropped: u32,
    failure: Option<String>,
}

// SAFETY : les objets D3D11 et les poignées ne sont touchés que par le thread
// qui possède le sink, un seul à la fois. Ils ne sont liés à aucun thread.
unsafe impl Send for SpoutSink {}

impl SpoutSink {
    /// Publie le sender sur l'adaptateur `vendor`/`device` de wgpu. Le nom est
    /// réservé ici : un conflit se dit à l'ouverture, pas à la première frame.
    pub fn new(
        name: &str,
        (w, h): (u32, u32),
        vendor: u32,
        device: u32,
    ) -> Result<Self, SpoutError> {
        if name.is_empty() || name.len() >= NAME_LEN {
            return Err(SpoutError::InvalidName);
        }
        let (d3d, context) = device_on(vendor, device)?;
        let texture = shared_texture(&d3d, w, h)?;
        let query = {
            let desc = D3D11_QUERY_DESC {
                Query: D3D11_QUERY_EVENT,
                MiscFlags: 0,
            };
            let mut query = None;
            // SAFETY : `desc` et la sortie vivent le temps de l'appel.
            unsafe { d3d.CreateQuery(&desc, Some(&mut query))? };
            query.ok_or_else(|| SpoutError::System("pas de requête".into()))?
        };

        let (names, slots) = match Mapping::open_registry(FILE_MAP_ALL_ACCESS.0) {
            Ok(trouve) => trouve,
            Err(_) => (
                Mapping::create("SpoutSenderNames", MAX_SENDERS * NAME_LEN)?,
                MAX_SENDERS,
            ),
        };
        // SAFETY : `slots` places de `NAME_LEN` octets tiennent dans la vue.
        let places: Vec<String> = (0..slots)
            .map(|i| unsafe { read_name(names.view.add(i * NAME_LEN)) })
            .collect();
        let vivant = Mapping::open(name, FILE_MAP_READ.0, INFO_LEN).is_ok();
        let place = claim_slot(&places, name, vivant)?;

        let active = Mapping::create("ActiveSenderName", NAME_LEN)?;
        let info = Mapping::create(name, INFO_LEN)?;
        let (m, s) = (
            cstr(&format!("{name}SpoutAccessMutex")),
            cstr(&format!("{name}_Count_Semaphore")),
        );
        // SAFETY : les noms finissent par un zéro et vivent le temps des appels ;
        // les vues font au moins `NAME_LEN` octets.
        let (mutex, semaphore) = unsafe {
            write_name(names.view.add(place * NAME_LEN), name);
            write_name(active.view, name);
            (
                CreateMutexA(None, false, PCSTR(m.as_ptr()))?,
                CreateSemaphoreA(None, 0, i32::MAX, PCSTR(s.as_ptr()))?,
            )
        };
        let mut sink = Self {
            name: name.to_string(),
            d3d,
            context,
            texture,
            query,
            size: (w, h),
            names,
            slots,
            _active: active,
            info,
            mutex,
            semaphore,
            dropped: 0,
            failure: None,
        };
        sink.write_info()?;
        Ok(sink)
    }

    /// Le bloc qu'un récepteur lit pour trouver la texture.
    fn write_info(&mut self) -> Result<(), SpoutError> {
        // SAFETY : l'appel ne garde aucun pointeur.
        let poignee = unsafe { self.texture.cast::<IDXGIResource>()?.GetSharedHandle()? };
        let (w, h) = self.size;
        // SAFETY : la vue fait `INFO_LEN` octets, et cinq `u32` y tiennent.
        unsafe {
            std::ptr::write_bytes(self.info.view, 0, INFO_LEN);
            for (i, v) in [poignee.0 as usize as u32, w, h, DXGI_BGRA, 0]
                .iter()
                .enumerate()
            {
                std::ptr::write_unaligned(self.info.view.add(i * 4) as *mut u32, *v);
            }
        }
        Ok(())
    }

    /// Une autre taille : une autre texture, sous le même nom. Le récepteur la
    /// retrouve à sa poignée (§8.5).
    fn resize(&mut self, w: u32, h: u32) -> Result<(), SpoutError> {
        self.texture = shared_texture(&self.d3d, w, h)?;
        self.size = (w, h);
        self.write_info()
    }

    /// Attend que le GPU ait exécuté la copie : après un simple `Flush`, un
    /// récepteur sur un autre device relit l'image précédente (§8.5).
    fn wait_gpu(&self) -> Result<(), SpoutError> {
        let requete: ID3D11Asynchronous = self.query.cast()?;
        // SAFETY : `fini` est une sortie locale de la taille annoncée.
        unsafe {
            self.context.End(&requete);
            self.context.Flush();
            let mut fini = 0i32;
            while fini == 0 {
                self.context.GetData(
                    &requete,
                    Some(&mut fini as *mut i32 as *mut c_void),
                    size_of::<i32>() as u32,
                    0,
                )?;
                if fini == 0 {
                    std::thread::yield_now();
                }
            }
        }
        Ok(())
    }

    fn publish(&mut self, frame: &CpuFrame) -> Result<(), SpoutError> {
        if (frame.width, frame.height) != self.size {
            self.resize(frame.width, frame.height)?;
        }
        // SAFETY : la poignée vit autant que le sink.
        if unsafe { WaitForSingleObject(self.mutex, MUTEX_TIMEOUT_MS) } != WAIT_OBJECT_0 {
            self.dropped = self.dropped.saturating_add(1);
            return Ok(());
        }
        let ecrit = self.upload(frame).and_then(|()| self.wait_gpu());
        // SAFETY : le mutex est à nous depuis l'attente réussie ci-dessus.
        unsafe {
            let _ = ReleaseMutex(self.mutex);
            if ecrit.is_ok() {
                let _ = ReleaseSemaphore(self.semaphore, 1, None);
            }
        }
        ecrit
    }

    fn upload(&self, frame: &CpuFrame) -> Result<(), SpoutError> {
        let attendu = (frame.width * frame.height * BYTES_PER_PIXEL) as usize;
        if frame.pixels.len() != attendu {
            return Err(SpoutError::System("image de taille incohérente".into()));
        }
        let texture: ID3D11Resource = self.texture.cast()?;
        // SAFETY : `pixels` couvre toute la texture, avec ce pas de ligne.
        unsafe {
            self.context.UpdateSubresource(
                &texture,
                0,
                None,
                frame.pixels.as_ptr() as *const c_void,
                frame.width * BYTES_PER_PIXEL,
                0,
            );
        }
        Ok(())
    }
}

impl CpuSink for SpoutSink {
    fn submit(&mut self, frame: &CpuFrame) {
        if frame.layout != PixelLayout::BgraOpaque {
            self.failure = Some("Spout attend une image BGRA".into());
            return;
        }
        if let Err(e) = self.publish(frame) {
            self.failure = Some(e.to_string());
        }
    }

    fn status(&self) -> SinkStatus {
        match (&self.failure, self.dropped) {
            (Some(e), _) => SinkStatus::Failed(e.clone()),
            (None, 0) => SinkStatus::Active,
            (None, dropped) => SinkStatus::Degraded { dropped },
        }
    }
}

impl Drop for SpoutSink {
    fn drop(&mut self) {
        // SAFETY : `slots` places tiennent dans la vue ; les poignées sont à nous.
        unsafe {
            for i in 0..self.slots {
                let place = self.names.view.add(i * NAME_LEN);
                if read_name(place) == self.name {
                    std::ptr::write_bytes(place, 0, NAME_LEN);
                    break;
                }
            }
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.semaphore);
        }
    }
}

/// Le nom de chaque sender publié sur la machine.
pub fn sender_names() -> Vec<String> {
    match Mapping::open_registry(FILE_MAP_READ.0) {
        // SAFETY : `slots` places de `NAME_LEN` octets tiennent dans la vue.
        Ok((carte, slots)) => (0..slots)
            .map(|i| unsafe { read_name(carte.view.add(i * NAME_LEN)) })
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_USAGE_STAGING,
    };

    fn place(noms: &[&str]) -> Vec<String> {
        noms.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn un_nom_neuf_prend_la_premiere_place_libre() {
        assert_eq!(claim_slot(&place(&["a", "", ""]), "b", false), Ok(1));
    }

    #[test]
    fn un_nom_vivant_est_refuse() {
        assert_eq!(
            claim_slot(&place(&["a", ""]), "a", true),
            Err(SpoutError::NameTaken("a".into()))
        );
    }

    #[test]
    fn un_nom_orphelin_reprend_sa_place() {
        assert_eq!(claim_slot(&place(&["", "a"]), "a", false), Ok(1));
    }

    #[test]
    fn un_registre_plein_est_refuse() {
        assert_eq!(
            claim_slot(&place(&["a"]), "b", false),
            Err(SpoutError::RegistryFull)
        );
    }

    #[test]
    fn un_nom_vide_ou_trop_long_est_refuse_a_l_ouverture() {
        assert!(matches!(
            SpoutSink::new("", (16, 16), 0, 0),
            Err(SpoutError::InvalidName)
        ));
        assert!(matches!(
            SpoutSink::new(&"x".repeat(NAME_LEN), (16, 16), 0, 0),
            Err(SpoutError::InvalidName)
        ));
    }

    fn frame(w: u32, h: u32, bgra: [u8; 4], index: u64) -> CpuFrame {
        CpuFrame {
            pixels: Arc::from(bgra.repeat((w * h) as usize)),
            layout: PixelLayout::BgraOpaque,
            width: w,
            height: h,
            index,
            t: 0.0,
        }
    }

    /// Un nom propre à ce test, pour ne gêner aucun sender de la machine.
    fn nom(suffixe: &str) -> String {
        format!("frogenx-test-{}-{suffixe}", std::process::id())
    }

    /// Lit le sender comme le ferait un récepteur, dans un second device.
    struct Recepteur {
        d3d: ID3D11Device,
        context: ID3D11DeviceContext,
        name: String,
    }

    impl Recepteur {
        fn new(name: &str) -> Option<Self> {
            let (d3d, context) = device_on(0, 0).ok()?;
            Some(Self {
                d3d,
                context,
                name: name.to_string(),
            })
        }

        fn info(&self) -> [u32; 4] {
            let carte = Mapping::open(&self.name, FILE_MAP_READ.0, INFO_LEN).unwrap();
            let v =
                |i: usize| unsafe { std::ptr::read_unaligned(carte.view.add(i * 4) as *const u32) };
            [v(0), v(1), v(2), v(3)]
        }

        /// Le compteur de frames, à la façon du SDK Spout.
        fn count(&self) -> i32 {
            let n = cstr(&format!("{}_Count_Semaphore", self.name));
            unsafe {
                let s = CreateSemaphoreA(None, 0, i32::MAX, PCSTR(n.as_ptr())).unwrap();
                let compte = if WaitForSingleObject(s, 0) == WAIT_OBJECT_0 {
                    let mut avant = 0;
                    let _ = ReleaseSemaphore(s, 1, Some(&mut avant));
                    avant + 1
                } else {
                    0
                };
                let _ = CloseHandle(s);
                compte
            }
        }

        /// Le premier pixel de la texture partagée, en BGRA.
        fn first_pixel(&self) -> [u8; 4] {
            let [poignee, w, h, _] = self.info();
            unsafe {
                let mut partagee: Option<ID3D11Texture2D> = None;
                self.d3d
                    .OpenSharedResource(HANDLE(poignee as usize as *mut c_void), &mut partagee)
                    .unwrap();
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut lecture = None;
                self.d3d
                    .CreateTexture2D(&desc, None, Some(&mut lecture))
                    .unwrap();
                let lecture: ID3D11Resource = lecture.unwrap().cast().unwrap();
                let partagee: ID3D11Resource = partagee.unwrap().cast().unwrap();
                self.context.CopyResource(&lecture, &partagee);
                let mut carte = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&lecture, 0, D3D11_MAP_READ, 0, Some(&mut carte))
                    .unwrap();
                let px = *(carte.pData as *const [u8; 4]);
                self.context.Unmap(&lecture, 0);
                px
            }
        }
    }

    /// Ouvre un sender et son récepteur. `None` sans Direct3D 11 (§24.3).
    fn paire(suffixe: &str, taille: (u32, u32)) -> Option<(SpoutSink, Recepteur)> {
        let nom = nom(suffixe);
        let Ok(sink) = SpoutSink::new(&nom, taille, 0, 0) else {
            eprintln!("Direct3D 11 absent, test sauté (§24.3)");
            return None;
        };
        Some((sink, Recepteur::new(&nom)?))
    }

    #[test]
    fn le_sender_s_inscrit_au_registre_et_s_en_retire() {
        let Some((sink, _)) = paire("registre", (16, 16)) else {
            return;
        };
        let nom = nom("registre");
        assert!(sender_names().contains(&nom));
        drop(sink);
        assert!(!sender_names().contains(&nom));
    }

    #[test]
    fn le_bloc_d_informations_decrit_la_texture() {
        let Some((_sink, r)) = paire("infos", (64, 32)) else {
            return;
        };
        let [poignee, w, h, format] = r.info();
        assert_ne!(poignee, 0);
        assert_eq!((w, h, format), (64, 32, DXGI_BGRA));
    }

    #[test]
    fn un_second_device_lit_l_image_publiee() {
        let Some((mut sink, r)) = paire("image", (16, 16)) else {
            return;
        };
        sink.submit(&frame(16, 16, [10, 20, 30, 255], 0));
        assert_eq!(sink.status(), SinkStatus::Active);
        assert_eq!(r.first_pixel(), [10, 20, 30, 255]);
    }

    #[test]
    fn chaque_frame_incremente_le_compteur_du_recepteur() {
        let Some((mut sink, r)) = paire("compteur", (16, 16)) else {
            return;
        };
        for i in 0..3 {
            sink.submit(&frame(16, 16, [0, 0, 0, 255], i));
        }
        assert_eq!(r.count(), 3);
    }

    #[test]
    fn la_derniere_frame_est_visible_des_son_annonce() {
        let Some((mut sink, r)) = paire("fraicheur", (16, 16)) else {
            return;
        };
        for v in 0..40u8 {
            sink.submit(&frame(16, 16, [v, 0, 0, 255], v as u64));
            assert_eq!(r.first_pixel()[0], v, "image précédente relue");
        }
    }

    #[test]
    fn une_autre_taille_change_de_texture_sous_le_meme_nom() {
        let Some((mut sink, r)) = paire("taille", (16, 16)) else {
            return;
        };
        sink.submit(&frame(16, 16, [1, 1, 1, 255], 0));
        let [avant, ..] = r.info();
        sink.submit(&frame(32, 8, [2, 2, 2, 255], 1));
        let [apres, w, h, _] = r.info();
        assert_ne!(avant, apres, "nouvelle poignée");
        assert_eq!((w, h), (32, 8));
        assert_eq!(r.first_pixel(), [2, 2, 2, 255]);
    }

    #[test]
    fn deux_senders_ne_peuvent_pas_porter_le_meme_nom() {
        let Some((_sink, _)) = paire("double", (16, 16)) else {
            return;
        };
        assert!(matches!(
            SpoutSink::new(&nom("double"), (16, 16), 0, 0),
            Err(SpoutError::NameTaken(_))
        ));
    }

    #[test]
    fn une_image_rgba_met_le_flux_en_echec_sans_panique() {
        let Some((mut sink, _)) = paire("rgba", (16, 16)) else {
            return;
        };
        let mut f = frame(16, 16, [0, 0, 0, 255], 0);
        f.layout = PixelLayout::RgbaStraight;
        sink.submit(&f);
        assert!(matches!(sink.status(), SinkStatus::Failed(_)));
    }

    #[test]
    fn une_image_tronquee_met_le_flux_en_echec_sans_panique() {
        let Some((mut sink, _)) = paire("tronquee", (16, 16)) else {
            return;
        };
        let mut f = frame(16, 16, [0, 0, 0, 255], 0);
        f.pixels = Arc::from(vec![0u8; 4]);
        sink.submit(&f);
        assert!(matches!(sink.status(), SinkStatus::Failed(_)));
    }
}
