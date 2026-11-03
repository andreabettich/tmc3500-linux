//! Job settings and the rules of the Windows driver dialog.
use crate::cupsopts::{parse_length_mm10, tokenize, Unit};
use crate::error::OptError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSize {
    /// The driver's predefined "108 x 174.8 mm - Die-cut Label".
    Label108x1748,
    /// Width and length in 0.1 mm.
    Custom { width: u32, length: u32 },
}

impl MediaSize {
    pub fn mm10(self) -> (u32, u32) {
        match self {
            Self::Label108x1748 => (1080, 1748),
            Self::Custom { width, length } => (width, length),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaForm {
    DieCutGap,
    DieCutBlackMark,
    ContinuousLabel,
    ContinuousPaper,
    ContinuousPaperBlackMark,
}

impl MediaForm {
    /// Labels on a liner (edges added to the width).
    pub fn is_label(self) -> bool {
        matches!(
            self,
            Self::DieCutGap | Self::DieCutBlackMark | Self::ContinuousLabel
        )
    }
    pub fn is_die_cut(self) -> bool {
        matches!(self, Self::DieCutGap | Self::DieCutBlackMark)
    }
    pub fn is_continuous(self) -> bool {
        !self.is_die_cut()
    }
    /// Value of the REMOTE1 `EX .. 24 nn` media-form command.
    pub fn ex24_code(self) -> u8 {
        match self {
            Self::DieCutGap => 2,
            Self::DieCutBlackMark => 3,
            Self::ContinuousLabel | Self::ContinuousPaper => 0,
            Self::ContinuousPaperBlackMark => 1,
        }
    }
    fn supports_saving(self) -> bool {
        matches!(self, Self::ContinuousLabel | Self::ContinuousPaper)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSaving {
    None,
    Lower,
    LowerUpper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coating {
    Plain,
    Synthetic,
    Matte,
    Wristband,
    Glossy,
}

impl Coating {
    /// Byte 8 of the `SN` media block.
    pub fn sn_code(self) -> u8 {
        match self {
            Self::Plain => 1,
            Self::Matte => 3,
            Self::Synthetic => 4,
            Self::Wristband => 5,
            Self::Glossy => 6,
        }
    }
    fn allows(self, q: Quality) -> bool {
        match self {
            Self::Plain => q == Quality::Speed,
            Self::Wristband => q != Quality::Quality2,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Speed,
    Quality1,
    Quality2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterPrint {
    CutEveryPage,
    CutLastPage,
    CutEveryN(u8),
    PeelOff,
    CutPosition,
    PrintEnd,
}

impl AfterPrint {
    pub fn is_cut(self) -> bool {
        matches!(
            self,
            Self::CutEveryPage | Self::CutLastPage | Self::CutEveryN(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    None,
    PageEnd,
    JobEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vacuum {
    Auto,
    Manual(u8),
}

impl Vacuum {
    /// Byte 18 of the `SN` media block: the suction value of slider position 0..=9 (Windows
    /// captures of every position), 0xff for automatic.
    pub fn sn_byte(self) -> u8 {
        const SUCTION: [u8; 10] = [30, 40, 50, 60, 65, 70, 75, 80, 85, 90];
        match self {
            Self::Auto => 0xff,
            Self::Manual(n) => SUCTION[usize::from(n.min(9))],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlackMode {
    Rich,
    KOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Preferred,
    Standard,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gamma {
    G18,
    G22,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSettings {
    pub mode: ColorMode,
    pub gamma: Gamma,
    pub ink_adjust: bool,
    pub ink_bkcmy: i8,
    pub ink_bk: i8,
    pub brightness: i8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobConfig {
    pub media: MediaSize,
    pub form: MediaForm,
    pub saving: MediaSaving,
    pub coating: Coating,
    pub quality: Quality,
    pub banding_reduction: bool,
    pub after_print: AfterPrint,
    pub notify: Notify,
    pub borderless: bool,
    pub bidirectional: bool,
    pub vacuum: Vacuum,
    pub rotate180: bool,
    /// Print position adjustment in 0.1 mm (signed).
    pub pos_adj_h: i32,
    pub pos_adj_v: i32,
    pub skip_blank: bool,
    pub black: BlackMode,
    pub color: ColorSettings,
    pub copies: u16,
    pub collate: bool,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            media: MediaSize::Label108x1748,
            form: MediaForm::DieCutGap,
            saving: MediaSaving::None,
            coating: Coating::Matte,
            quality: Quality::Speed,
            banding_reduction: true,
            after_print: AfterPrint::CutEveryPage,
            notify: Notify::None,
            borderless: true,
            bidirectional: true,
            vacuum: Vacuum::Auto,
            rotate180: false,
            pos_adj_h: 0,
            pos_adj_v: 0,
            skip_blank: false,
            black: BlackMode::Rich,
            color: ColorSettings {
                mode: ColorMode::Preferred,
                gamma: Gamma::G22,
                ink_adjust: false,
                ink_bkcmy: 0,
                ink_bk: 0,
                brightness: 0,
            },
            copies: 1,
            collate: false,
        }
    }
}

const BOOL_KEYS: &[&str] = &[
    "TmBandingReduction",
    "TmBorderless",
    "TmBidirectional",
    "TmRotate180",
    "TmSkipBlank",
    "TmInkAdjust",
    "Collate",
];

fn bad(key: &str, value: &str) -> OptError {
    OptError::BadValue {
        key: key.into(),
        value: value.into(),
    }
}

fn range(key: &str, value: &str, allowed: &str) -> OptError {
    OptError::OutOfRange {
        key: key.into(),
        value: value.into(),
        allowed: allowed.into(),
    }
}

fn parse_bool(key: &str, v: &str) -> Result<bool, OptError> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(bad(key, v)),
    }
}

fn parse_int(key: &str, v: &str, lo: i64, hi: i64) -> Result<i64, OptError> {
    if v.is_empty() || v.len() > 12 {
        return Err(bad(key, v));
    }
    let n: i64 = v.parse().map_err(|_| bad(key, v))?;
    if n < lo || n > hi {
        return Err(range(key, v, &format!("{lo}..={hi}")));
    }
    Ok(n)
}

fn pick<T: Copy>(key: &str, v: &str, table: &[(&str, T)]) -> Result<T, OptError> {
    table
        .iter()
        .find(|(name, _)| *name == v)
        .map(|(_, t)| *t)
        .ok_or_else(|| bad(key, v))
}

fn parse_page_size(v: &str) -> Result<MediaSize, OptError> {
    const KEY: &str = "PageSize";
    if v == "Label108x174.8mm" {
        return Ok(MediaSize::Label108x1748);
    }
    let body = v.strip_prefix("Custom.").ok_or_else(|| bad(KEY, v))?;
    let (body, unit) = [
        ("in", Unit::In),
        ("mm", Unit::Mm),
        ("cm", Unit::Cm),
        ("pt", Unit::Pt),
    ]
    .iter()
    .find_map(|(sfx, u)| body.strip_suffix(sfx).map(|b| (b, *u)))
    .unwrap_or((body, Unit::Pt));
    let (w, l) = body.split_once('x').ok_or_else(|| bad(KEY, v))?;
    let w = parse_length_mm10(KEY, w, unit)?;
    let l = parse_length_mm10(KEY, l, unit)?;
    let to_u32 = |x: i64| u32::try_from(x).map_err(|_| range(KEY, v, "positive size"));
    Ok(MediaSize::Custom {
        width: to_u32(w)?,
        length: to_u32(l)?,
    })
}

fn parse_pos(key: &str, v: &str, max_mm10: i64, allowed: &str) -> Result<i32, OptError> {
    let n = parse_length_mm10(key, v, Unit::In)?;
    if n.abs() > max_mm10 {
        return Err(range(key, v, allowed));
    }
    Ok(n as i32)
}

impl JobConfig {
    /// Build a validated configuration from the CUPS option string and copy count.
    pub fn from_cups(options: &str, copies: u32) -> Result<JobConfig, OptError> {
        let mut c = JobConfig::default();
        if !(1..=9999).contains(&copies) {
            return Err(range("copies", &copies.to_string(), "1..=9999"));
        }
        c.copies = copies as u16;
        let mut quality_explicit_banding: Option<bool> = None;
        let mut after: Option<String> = None;
        let mut interval: u8 = 1;
        for (k, v) in tokenize(options)? {
            let (k, v) = match k.strip_prefix("no") {
                Some(base) if BOOL_KEYS.contains(&base) && v == "true" => {
                    (base.to_string(), "false".to_string())
                }
                // A negated driver option must name a known boolean and carry no value;
                // otherwise a typo would be silently ignored.
                Some(base) if base.starts_with("Tm") => {
                    return Err(if BOOL_KEYS.contains(&base) {
                        bad(&k, &v)
                    } else {
                        OptError::UnknownOption(k)
                    });
                }
                _ => (k, v),
            };
            let (k, v) = (k.as_str(), v.as_str());
            match k {
                "PageSize" => c.media = parse_page_size(v)?,
                "TmMediaForm" => {
                    use MediaForm::*;
                    c.form = pick(
                        k,
                        v,
                        &[
                            ("DieCutGap", DieCutGap),
                            ("DieCutBlackMark", DieCutBlackMark),
                            ("ContinuousLabel", ContinuousLabel),
                            ("ContinuousPaper", ContinuousPaper),
                            ("ContinuousPaperBlackMark", ContinuousPaperBlackMark),
                        ],
                    )?
                }
                "TmMediaSaving" => {
                    c.saving = pick(
                        k,
                        v,
                        &[
                            ("None", MediaSaving::None),
                            ("Lower", MediaSaving::Lower),
                            ("LowerUpper", MediaSaving::LowerUpper),
                        ],
                    )?
                }
                "TmCoating" => {
                    use Coating::*;
                    c.coating = pick(
                        k,
                        v,
                        &[
                            ("Plain", Plain),
                            ("Synthetic", Synthetic),
                            ("Matte", Matte),
                            ("Wristband", Wristband),
                            ("Glossy", Glossy),
                        ],
                    )?
                }
                "TmQuality" => {
                    c.quality = pick(
                        k,
                        v,
                        &[
                            ("Speed", Quality::Speed),
                            ("Quality1", Quality::Quality1),
                            ("Quality2", Quality::Quality2),
                        ],
                    )?
                }
                "TmBandingReduction" => quality_explicit_banding = Some(parse_bool(k, v)?),
                "TmAfterPrint" => {
                    pick(
                        k,
                        v,
                        &[
                            ("CutEveryPage", ()),
                            ("CutLastPage", ()),
                            ("CutEveryN", ()),
                            ("PeelOff", ()),
                            ("CutPosition", ()),
                            ("PrintEnd", ()),
                        ],
                    )?;
                    after = Some(v.to_string());
                }
                "TmCutInterval" => interval = parse_int(k, v, 1, 255)? as u8,
                "TmNotify" => {
                    c.notify = pick(
                        k,
                        v,
                        &[
                            ("None", Notify::None),
                            ("PageEnd", Notify::PageEnd),
                            ("JobEnd", Notify::JobEnd),
                        ],
                    )?
                }
                "TmBorderless" => c.borderless = parse_bool(k, v)?,
                "TmBidirectional" => c.bidirectional = parse_bool(k, v)?,
                "TmRotate180" => c.rotate180 = parse_bool(k, v)?,
                "TmSkipBlank" => c.skip_blank = parse_bool(k, v)?,
                "TmVacuum" => {
                    c.vacuum = if v == "Auto" {
                        Vacuum::Auto
                    } else {
                        Vacuum::Manual(parse_int(k, v, 0, 9)? as u8)
                    }
                }
                "TmPosAdjH" => c.pos_adj_h = parse_pos(k, v, 1039, "-4.09in..=4.09in")?,
                "TmPosAdjV" => c.pos_adj_v = parse_pos(k, v, 11176, "-44in..=44in")?,
                "TmBlack" => {
                    c.black = pick(
                        k,
                        v,
                        &[("Rich", BlackMode::Rich), ("KOnly", BlackMode::KOnly)],
                    )?
                }
                "TmColorMode" => {
                    c.color.mode = pick(
                        k,
                        v,
                        &[
                            ("Preferred", ColorMode::Preferred),
                            ("Standard", ColorMode::Standard),
                            ("Off", ColorMode::Off),
                        ],
                    )?
                }
                "TmGamma" => {
                    c.color.gamma = pick(k, v, &[("1.8", Gamma::G18), ("2.2", Gamma::G22)])?
                }
                "TmInkAdjust" => c.color.ink_adjust = parse_bool(k, v)?,
                "TmInkBkCMY" => c.color.ink_bkcmy = parse_int(k, v, -6, 0)? as i8,
                "TmInkK" => c.color.ink_bk = parse_int(k, v, -6, 0)? as i8,
                "TmBrightness" => c.color.brightness = parse_int(k, v, -2, 2)? as i8,
                "Collate" => c.collate = parse_bool(k, v)?,
                other if other.starts_with("Tm") => {
                    return Err(OptError::UnknownOption(other.into()))
                }
                _ => {}
            }
        }
        c.after_print = match after.as_deref() {
            None | Some("CutEveryPage") => AfterPrint::CutEveryPage,
            Some("CutLastPage") => AfterPrint::CutLastPage,
            Some("CutEveryN") => AfterPrint::CutEveryN(interval),
            Some("PeelOff") => AfterPrint::PeelOff,
            Some("CutPosition") => AfterPrint::CutPosition,
            _ => AfterPrint::PrintEnd,
        };
        c.apply_rules(quality_explicit_banding)?;
        Ok(c)
    }

    fn apply_rules(&mut self, banding: Option<bool>) -> Result<(), OptError> {
        if let MediaSize::Custom { width, length } = &mut self.media {
            let (wmin, wtxt) = if self.form.is_label() {
                (254, "1.00in..=4.25in")
            } else {
                (300, "1.18in..=4.25in")
            };
            if !(wmin..=1080).contains(width) {
                return Err(range("PageSize", &format!("width {width} (0.1mm)"), wtxt));
            }
            if !(79..=11176).contains(length) {
                return Err(range(
                    "PageSize",
                    &format!("length {length} (0.1mm)"),
                    "0.31in..=44in",
                ));
            }
            *length = (*length).max(80);
        }
        if !self.coating.allows(self.quality) {
            return Err(OptError::Conflict(format!(
                "{:?} coating does not allow {:?}",
                self.coating, self.quality
            )));
        }
        self.banding_reduction = match (self.coating, self.quality) {
            (Coating::Plain, _) | (_, Quality::Quality1) => false,
            (_, Quality::Quality2) => true,
            _ => banding.unwrap_or(true),
        };
        if self.after_print.is_cut() && self.media.mm10().1 < 150 {
            self.after_print = AfterPrint::PeelOff;
        }
        if !self.form.supports_saving() {
            self.saving = MediaSaving::None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(o: &str) -> JobConfig {
        JobConfig::from_cups(o, 1).unwrap()
    }

    #[test]
    fn defaults_match_windows_driver() {
        let c = cfg("");
        assert_eq!(c, JobConfig::default());
        assert_eq!(c.media, MediaSize::Label108x1748);
        assert_eq!(c.form, MediaForm::DieCutGap);
        assert_eq!(c.coating, Coating::Matte);
        assert_eq!(c.quality, Quality::Speed);
        assert!(c.banding_reduction && c.borderless && c.bidirectional);
        assert_eq!(c.after_print, AfterPrint::CutEveryPage);
        assert_eq!(c.vacuum, Vacuum::Auto);
        assert_eq!((c.copies, c.collate), (1, false));
    }

    #[test]
    fn parses_custom_page_sizes_in_any_unit() {
        let want = MediaSize::Custom {
            width: 1080,
            length: 1748,
        };
        assert_eq!(cfg("PageSize=Custom.4.25x6.88in").media, want);
        assert_eq!(cfg("PageSize=Custom.306x495.36").media, want);
        assert_eq!(cfg("PageSize=Custom.108x174.8mm").media, want);
        assert_eq!(
            cfg("PageSize=Custom.1.00x0.31in").media,
            MediaSize::Custom {
                width: 254,
                length: 80
            }
        );
    }

    #[test]
    fn ignores_standard_cups_options_and_last_key_wins() {
        let c = cfg("job-uuid=urn:x orientation-requested=4 fit-to-page=true TmCoating=Plain TmCoating=Glossy noTmBidirectional");
        assert_eq!(c.coating, Coating::Glossy);
        assert!(!c.bidirectional);
    }

    #[test]
    fn rejects_unknown_tm_options_and_bad_values() {
        assert!(matches!(
            JobConfig::from_cups("TmFoo=1", 1),
            Err(OptError::UnknownOption(_))
        ));
        assert!(matches!(
            JobConfig::from_cups("TmCoating=Shiny", 1),
            Err(OptError::BadValue { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("TmCutInterval=256", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("PageSize=Custom.5x6in", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("PageSize=Custom.4x0.2in", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("PageSize=Custom.1.1x2in TmMediaForm=ContinuousPaper", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("PageSize=Letter", 1),
            Err(OptError::BadValue { .. })
        ));
    }

    #[test]
    fn applies_dialog_rules() {
        assert!(matches!(
            JobConfig::from_cups("TmCoating=Plain TmQuality=Quality1", 1),
            Err(OptError::Conflict(_))
        ));
        assert!(matches!(
            JobConfig::from_cups("TmCoating=Wristband TmQuality=Quality2", 1),
            Err(OptError::Conflict(_))
        ));
        assert!(!cfg("TmCoating=Plain").banding_reduction);
        assert!(!cfg("TmQuality=Quality1").banding_reduction);
        assert!(cfg("TmQuality=Quality2 TmBandingReduction=False").banding_reduction);
        assert!(!cfg("TmBandingReduction=False").banding_reduction);
        assert_eq!(
            cfg("PageSize=Custom.1x0.5in").after_print,
            AfterPrint::PeelOff
        );
        assert_eq!(
            cfg("PageSize=Custom.1x0.5in TmAfterPrint=PrintEnd").after_print,
            AfterPrint::PrintEnd
        );
        assert_eq!(cfg("TmMediaSaving=Lower").saving, MediaSaving::None);
        assert_eq!(
            cfg("TmMediaForm=ContinuousLabel TmMediaSaving=Lower").saving,
            MediaSaving::Lower
        );
        assert_eq!(
            cfg("TmAfterPrint=CutEveryN TmCutInterval=5").after_print,
            AfterPrint::CutEveryN(5)
        );
    }

    #[test]
    fn vacuum_slider_positions_map_to_the_windows_suction_values() {
        // captures 610-612 (session 1) and 730-736 (session 2): the logged Suction value
        let want = [30, 40, 50, 60, 65, 70, 75, 80, 85, 90];
        for (pos, w) in want.iter().enumerate() {
            assert_eq!(
                cfg(&format!("TmVacuum={pos}")).vacuum.sn_byte(),
                *w,
                "{pos}"
            );
        }
        assert_eq!(cfg("TmVacuum=Auto").vacuum.sn_byte(), 0xff);
        assert!(matches!(
            JobConfig::from_cups("TmVacuum=10", 1),
            Err(OptError::OutOfRange { .. })
        ));
    }

    #[test]
    fn copies_range_is_enforced() {
        assert!(matches!(
            JobConfig::from_cups("", 0),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("", 10_000),
            Err(OptError::OutOfRange { .. })
        ));
        assert_eq!(JobConfig::from_cups("Collate=True", 3).unwrap().copies, 3);
    }

    #[test]
    fn parses_position_and_color_options() {
        let c = cfg("TmPosAdjV=0.50in TmPosAdjH=-0.25 TmColorMode=Off TmGamma=1.8 TmInkAdjust=True TmInkBkCMY=-3 TmInkK=-6 TmBrightness=2 TmBlack=KOnly");
        assert_eq!((c.pos_adj_v, c.pos_adj_h), (127, -64));
        assert_eq!(
            c.color,
            ColorSettings {
                mode: ColorMode::Off,
                gamma: Gamma::G18,
                ink_adjust: true,
                ink_bkcmy: -3,
                ink_bk: -6,
                brightness: 2
            }
        );
        assert_eq!(c.black, BlackMode::KOnly);
        assert!(matches!(
            JobConfig::from_cups("TmPosAdjH=4.10in", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(
            JobConfig::from_cups("TmInkK=1", 1),
            Err(OptError::OutOfRange { .. })
        ));
    }

    #[test]
    fn media_form_codes() {
        use MediaForm::*;
        assert_eq!(
            [
                DieCutGap,
                DieCutBlackMark,
                ContinuousLabel,
                ContinuousPaper,
                ContinuousPaperBlackMark
            ]
            .map(MediaForm::ex24_code),
            [2, 3, 0, 0, 1]
        );
        assert_eq!(
            [
                Coating::Plain,
                Coating::Matte,
                Coating::Synthetic,
                Coating::Wristband,
                Coating::Glossy
            ]
            .map(Coating::sn_code),
            [1, 3, 4, 5, 6]
        );
    }

    #[test]
    fn negated_tm_options_are_validated_too() {
        for o in [
            "noTmBorderles",
            "noTmFoo",
            "noTmBorderless=false",
            "noTmCoating",
        ] {
            assert!(
                JobConfig::from_cups(o, 1).is_err(),
                "{o:?} should be rejected"
            );
        }
        assert!(!cfg("noTmBorderless").borderless);
        assert!(cfg("nothing=1 notice").borderless);
    }
}
