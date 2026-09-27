//! Who is this? The decision logic that turns embeddings into a name, or
//! into "unknown".
//!
//! The recognizer in [`crate::nn::edgeface`] turns an aligned face crop
//! into an *embedding*: [`EMBEDDING_LEN`] floats that describe the face and
//! not the picture. The numbers themselves mean nothing on their own; only
//! the angle between two embeddings does. Scale both to length 1 and their
//! dot product is the cosine of that angle, between -1 and 1. Two pictures
//! of the same person score roughly 0.5 to 0.9, two different people
//! roughly -0.1 to 0.3. That one number is all this module works with; it
//! never sees a pixel.
//!
//! Enrolling a person stores several embeddings, called *templates*, taken
//! at different head poses. A probe is compared with every template and the
//! best score counts ([`Person::best_similarity`]). Taking the maximum
//! works much better than averaging the templates into one vector: a face
//! turned 20 degrees to the left simply lands near the template that was
//! taken turned to the left, while the average of all poses is close to
//! none of them.
//!
//! A plain threshold on that best score is fragile. Scores drift up and
//! down together with the light, the exposure and the camera gain, so a
//! limit that is right indoors lets strangers in outdoors. The firmware
//! therefore also carries an *impostor bank*: a few hundred embeddings of
//! other people, computed on a computer and baked into the weights file.
//! The bank drifts with the same light as the probe, so the *difference*
//! between the two scores is far steadier than either score alone:
//!
//! ```text
//! accept  <=>  best person score >= accept
//!         and  best person score - best impostor score >= margin
//! ```
//!
//! The bank is large and lives in flash, so [`Gallery`] does not store it.
//! It is a borrowed slice, [`ImpostorBank`], that the caller passes to
//! [`Gallery::match_probe`]. That keeps [`Gallery`] free of lifetimes and
//! lets the application swap banks, for example a test bank, without
//! rebuilding the gallery.
//!
//! Two small filters sit around the decision, because a camera delivers
//! many frames per second and every one of them is noisy:
//!
//! - [`Fusion`] averages the last [`FUSION_FRAMES`] embeddings and
//!   re-normalizes the average. Averaging cancels part of the per-frame
//!   noise, which raises the score of the right person more than that of
//!   everyone else.
//! - [`Vote`] remembers the last [`VOTE_WINDOW`] decisions and only reports
//!   a new identity when the newest [`VOTE_AGREEMENT`] of them agree, so the
//!   name on the screen does not flicker between two people or blink to
//!   "unknown" for one bad frame.
//!
//! Nothing here allocates: every buffer is a fixed-size array inside a type
//! or a slice the caller lends us. [`Thresholds::DEFAULT`] holds placeholder
//! numbers until Step 7 calibrates them on the device.

use libm::sqrtf;

/// The number of values in one embedding.
///
/// This must match `nn::edgeface::EMBEDDING_LEN`. It is written down again
/// here so that reading this module needs no knowledge of the network; the
/// check below keeps the two in step.
pub const EMBEDDING_LEN: usize = 512;

/// Refuses to compile if the recognizer ever changes its output size.
const _: () = assert!(EMBEDDING_LEN == crate::nn::edgeface::EMBEDDING_LEN);

/// How many people a [`Gallery`] holds.
pub const MAX_PEOPLE: usize = 4;

/// How many templates one [`Person`] holds: one per head pose recorded
/// during enrollment.
pub const MAX_TEMPLATES: usize = 12;

/// How many bytes a person's name may use, in UTF-8.
pub const MAX_NAME: usize = 16;

/// How many embeddings [`Fusion`] averages before a match.
pub const FUSION_FRAMES: usize = 3;

/// How many recent decisions [`Vote`] remembers.
pub const VOTE_WINDOW: usize = 5;

/// How many of the [`VOTE_WINDOW`] remembered decisions must agree before
/// [`Vote`] reports a new identity.
pub const VOTE_AGREEMENT: usize = 3;

/// More than half of the window must agree, so at most one value can ever
/// reach the agreement count.
const _: () = assert!(2 * VOTE_AGREEMENT > VOTE_WINDOW);

/// A vector shorter than this counts as zero. Dividing by a length this
/// small would produce infinities instead of a direction.
const MIN_NORM: f32 = 1e-12;

/// The dot product of two equally long slices.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scale `values` to length 1. A vector that is zero, or shorter than
/// [`MIN_NORM`], is left alone.
fn normalize(values: &mut [f32; EMBEDDING_LEN]) {
    let norm = sqrtf(dot(values, values));
    if norm > MIN_NORM {
        for value in values.iter_mut() {
            *value /= norm;
        }
    }
}

/// One face as [`EMBEDDING_LEN`] numbers of length 1.
///
/// Copying one costs 2 KiB, so pass it by reference on hot paths.
#[derive(Clone, Copy, Debug)]
pub struct Embedding {
    /// The values, scaled to length 1 (or all zero).
    values: [f32; EMBEDDING_LEN],
}

impl Embedding {
    /// All zeros: the direction of no face. Its similarity to anything is
    /// 0, so it never wins a match. Useful to fill arrays.
    pub const ZERO: Self = Self {
        values: [0.0; EMBEDDING_LEN],
    };

    /// Copy the recognizer's raw output and scale it to length 1.
    ///
    /// A zero vector, or one too short to have a direction, stays as it is:
    /// the result is then [`Embedding::ZERO`], not a division by zero.
    ///
    /// # Panics
    ///
    /// When `raw` does not hold exactly [`EMBEDDING_LEN`] values.
    pub fn from_raw(raw: &[f32]) -> Self {
        assert_eq!(
            raw.len(),
            EMBEDDING_LEN,
            "an embedding has {EMBEDDING_LEN} values"
        );
        let mut values = [0.0f32; EMBEDDING_LEN];
        values.copy_from_slice(raw);
        normalize(&mut values);
        Self { values }
    }

    /// How similar two faces are, from -1 (opposite) to 1 (identical).
    ///
    /// Both vectors have length 1, so the dot product is the cosine of the
    /// angle between them. See the module documentation for the scores to
    /// expect from the same person and from different people.
    pub fn similarity(&self, other: &Self) -> f32 {
        dot(&self.values, &other.values)
    }

    /// The values, for code that writes an embedding to a file or a screen.
    pub fn values(&self) -> &[f32; EMBEDDING_LEN] {
        &self.values
    }
}

/// An enrolled person: a name and the templates recorded for them.
#[derive(Clone, Copy, Debug)]
pub struct Person {
    /// The name in UTF-8; only the first `name_len` bytes count.
    name: [u8; MAX_NAME],
    /// The number of bytes of `name` that are in use.
    name_len: usize,
    /// The recorded embeddings; only the first `template_count` count.
    templates: [Embedding; MAX_TEMPLATES],
    /// The number of templates recorded so far.
    template_count: usize,
}

impl Person {
    /// A nameless person with no templates, to fill a [`Gallery`] with.
    const EMPTY: Self = Self {
        name: [0; MAX_NAME],
        name_len: 0,
        templates: [Embedding::ZERO; MAX_TEMPLATES],
        template_count: 0,
    };

    /// The person's name.
    pub fn name(&self) -> &str {
        // The bytes were copied from a `&str`, so they are valid UTF-8.
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    /// The templates recorded so far, oldest first.
    pub fn templates(&self) -> &[Embedding] {
        &self.templates[..self.template_count]
    }

    /// Whether [`MAX_TEMPLATES`] templates are already recorded.
    pub fn is_full(&self) -> bool {
        self.template_count == MAX_TEMPLATES
    }

    /// Record one more template. Returns `false`, and keeps everything as
    /// it was, when the person is full.
    pub fn add_template(&mut self, e: Embedding) -> bool {
        if self.is_full() {
            return false;
        }
        self.templates[self.template_count] = e;
        self.template_count += 1;
        true
    }

    /// The best score of `probe` against any template, or `-1.0` when this
    /// person has no templates yet.
    pub fn best_similarity(&self, probe: &Embedding) -> f32 {
        self.templates()
            .iter()
            .map(|template| probe.similarity(template))
            .fold(-1.0f32, f32::max)
    }
}

/// Where the impostor bank's numbers come from: `f32` values, or the
/// `i8` form that `facekit bank` writes, which is a quarter of the size
/// and costs at most 0.0001 of a cosine.
#[derive(Clone, Copy, Debug)]
enum BankValues<'a> {
    /// Unit vectors as `f32`.
    Float(&'a [f32]),
    /// Unit vectors as `i8`, all with the same scale.
    Integer(&'a [i8], f32),
}

/// The faces of people the device does not know: a few hundred
/// embeddings, computed on a developer machine and carried in flash.
///
/// Recognition asks whether a face is more like the enrolled person than
/// like the closest of these strangers. That comparison survives a change
/// of light or camera gain, which moves every score together; a fixed
/// threshold does not.
///
/// The stored vectors are assumed to have length 1 already, as
/// `facekit bank` writes them.
#[derive(Clone, Copy, Debug)]
pub struct ImpostorBank<'a> {
    /// The vectors, `EMBEDDING_LEN` values each.
    values: BankValues<'a>,
}

impl<'a> ImpostorBank<'a> {
    /// A bank over `values`: one unit vector of [`EMBEDDING_LEN`] values
    /// after another.
    ///
    /// # Panics
    ///
    /// When the length is not a multiple of [`EMBEDDING_LEN`].
    pub fn new(values: &'a [f32]) -> Self {
        assert!(
            values.len().is_multiple_of(EMBEDDING_LEN),
            "an impostor bank holds whole embeddings"
        );
        Self {
            values: BankValues::Float(values),
        }
    }

    /// A bank over the `i8` form: `real = scale * stored`.
    ///
    /// # Panics
    ///
    /// When the length is not a multiple of [`EMBEDDING_LEN`].
    pub fn from_i8(values: &'a [i8], scale: f32) -> Self {
        assert!(
            values.len().is_multiple_of(EMBEDDING_LEN),
            "an impostor bank holds whole embeddings"
        );
        Self {
            values: BankValues::Integer(values, scale),
        }
    }

    /// An empty bank: every margin is then the person's own score plus
    /// one, so only the threshold decides.
    pub const EMPTY: Self = Self {
        values: BankValues::Float(&[]),
    };

    /// How many strangers the bank holds.
    pub fn len(&self) -> usize {
        match self.values {
            BankValues::Float(values) => values.len() / EMBEDDING_LEN,
            BankValues::Integer(values, _) => values.len() / EMBEDDING_LEN,
        }
    }

    /// Whether the bank is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The highest similarity between `probe` and any stranger, or
    /// `-1.0` when the bank is empty.
    pub fn best_similarity(&self, probe: &Embedding) -> f32 {
        let mut best = -1.0f32;
        match self.values {
            BankValues::Float(values) => {
                for member in values.chunks_exact(EMBEDDING_LEN) {
                    let mut sum = 0.0f32;
                    for (a, b) in probe.values().iter().zip(member) {
                        sum += a * b;
                    }
                    best = best.max(sum);
                }
            }
            BankValues::Integer(values, scale) => {
                for member in values.chunks_exact(EMBEDDING_LEN) {
                    let mut sum = 0.0f32;
                    for (a, &b) in probe.values().iter().zip(member) {
                        sum += a * f32::from(b);
                    }
                    best = best.max(sum * scale);
                }
            }
        }
        best
    }
}

/// The two numbers that decide a match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// The best score of an enrolled person must reach this.
    pub accept: f32,
    /// The best score must beat the best impostor score by this much.
    pub margin: f32,
}

impl Thresholds {
    /// The operating point `facekit calibrate` measured on Labeled Faces
    /// in the Wild: 120 people enrolled with five photos each, 1,351
    /// genuine attempts and 22,920 attempts by strangers, against a bank
    /// of 200 strangers.
    ///
    /// At `accept` 0.35 it recognized 98.8 percent of the genuine
    /// attempts and let through 0.02 percent of the strangers (five of
    /// 22,920). Raising `margin` to 0.10 cost two points of recognition
    /// and stopped no further stranger, so the margin stays at 0: a face
    /// must still beat the closest stranger in the bank, but by nothing
    /// in particular.
    ///
    /// Those photos are of one person on different days, which is harder
    /// than enrolling and recognizing in one sitting in front of the
    /// board, so these numbers are on the safe side. Step 10 checks them
    /// on the device.
    pub const DEFAULT: Self = Self {
        accept: 0.35,
        margin: 0.0,
    };
}

impl Default for Thresholds {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The outcome of a match. Both variants carry the numbers behind the
/// decision, so the user interface can show why a face was refused.
#[derive(Clone, Copy, Debug)]
pub enum Match<'a> {
    /// The probe is this person.
    Known {
        /// The person whose templates matched best.
        person: &'a Person,
        /// Their best score against the probe.
        score: f32,
        /// How far that score beat the best impostor.
        margin: f32,
    },
    /// Nobody in the gallery matched well enough.
    Unknown {
        /// The best score any enrolled person reached; `-1.0` when the
        /// gallery is empty or holds no templates.
        best_score: f32,
        /// How far that score beat the best impostor. It can be negative:
        /// then an impostor matched better than anyone enrolled.
        margin: f32,
    },
}

impl Match<'_> {
    /// Whether a person was recognized.
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known { .. })
    }

    /// The best score of any enrolled person, whatever the outcome.
    pub fn score(&self) -> f32 {
        match self {
            Self::Known { score, .. } => *score,
            Self::Unknown { best_score, .. } => *best_score,
        }
    }

    /// The name of the recognized person, or `None` when unknown.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Known { person, .. } => Some(person.name()),
            Self::Unknown { .. } => None,
        }
    }
}

/// The enrolled people, and the decision about a probe.
///
/// This is the whole enrollment database: a fixed array of [`MAX_PEOPLE`]
/// slots, each with room for [`MAX_TEMPLATES`] templates. It is large (see
/// `core::mem::size_of`), so the firmware keeps one of these in a static or
/// on the heap, never on a task stack.
#[derive(Clone, Copy, Debug)]
pub struct Gallery {
    /// The slots; only the first `count` hold a person.
    people: [Person; MAX_PEOPLE],
    /// The number of slots in use.
    count: usize,
}

impl Gallery {
    /// An empty gallery.
    pub fn new() -> Self {
        Self {
            people: [Person::EMPTY; MAX_PEOPLE],
            count: 0,
        }
    }

    /// The number of enrolled people.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether nobody is enrolled.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The enrolled people, in the order they were enrolled.
    pub fn people(&self) -> &[Person] {
        &self.people[..self.count]
    }

    /// Add a person with no templates yet and borrow them, so the caller
    /// can go on and record templates.
    ///
    /// Returns `None` when the gallery is full, when `name` is empty or
    /// longer than [`MAX_NAME`] bytes, or when that name is already
    /// enrolled. The name is the identity here, so it has to be unique.
    pub fn enroll(&mut self, name: &str) -> Option<&mut Person> {
        if self.count == MAX_PEOPLE
            || name.is_empty()
            || name.len() > MAX_NAME
            || self.person(name).is_some()
        {
            return None;
        }
        let slot = &mut self.people[self.count];
        *slot = Person::EMPTY;
        slot.name[..name.len()].copy_from_slice(name.as_bytes());
        slot.name_len = name.len();
        self.count += 1;
        Some(slot)
    }

    /// The person called `name`, if enrolled.
    pub fn person(&self, name: &str) -> Option<&Person> {
        self.people().iter().find(|person| person.name() == name)
    }

    /// The person called `name`, to add templates to.
    pub fn person_mut(&mut self, name: &str) -> Option<&mut Person> {
        self.people[..self.count]
            .iter_mut()
            .find(|person| person.name() == name)
    }

    /// Remove the person called `name`, with all their templates. Returns
    /// `false` when no such person is enrolled.
    ///
    /// The people behind them move up one slot, so any index kept from
    /// [`people`](Self::people), for example inside a [`Vote`], is stale
    /// afterwards: clear the vote after forgetting someone.
    pub fn forget(&mut self, name: &str) -> bool {
        let Some(index) = self.people().iter().position(|p| p.name() == name) else {
            return false;
        };
        for slot in index..self.count - 1 {
            self.people[slot] = self.people[slot + 1];
        }
        self.count -= 1;
        self.people[self.count] = Person::EMPTY;
        true
    }

    /// Decide who `probe` is.
    ///
    /// The best-matching person must reach `thresholds.accept` *and* beat
    /// the best impostor in `bank` by `thresholds.margin`; see the module
    /// documentation for why both tests are needed.
    pub fn match_probe(
        &self,
        probe: &Embedding,
        bank: &ImpostorBank<'_>,
        thresholds: &Thresholds,
    ) -> Match<'_> {
        let impostor = bank.best_similarity(probe);
        // The best-scoring person, or `None` when nobody is enrolled. On a
        // tie the person enrolled first wins.
        let best = self
            .people()
            .iter()
            .map(|person| (person, person.best_similarity(probe)))
            .fold(None::<(&Person, f32)>, |best, candidate| match best {
                Some(best) if best.1 >= candidate.1 => Some(best),
                _ => Some(candidate),
            });
        match best {
            Some((person, score))
                if score >= thresholds.accept && score - impostor >= thresholds.margin =>
            {
                Match::Known {
                    person,
                    score,
                    margin: score - impostor,
                }
            }
            Some((_, score)) => Match::Unknown {
                best_score: score,
                margin: score - impostor,
            },
            None => Match::Unknown {
                best_score: -1.0,
                margin: -1.0 - impostor,
            },
        }
    }
}

impl Default for Gallery {
    fn default() -> Self {
        Self::new()
    }
}

/// The average of the last [`FUSION_FRAMES`] embeddings.
///
/// Every frame is noisy in its own way, and the noise of several frames
/// partly cancels. The average of embeddings of the same face is closer to
/// that person's templates than any single frame is, which widens the gap
/// to everyone else. The average is re-normalized, so the result is a
/// proper embedding again.
#[derive(Clone, Copy, Debug)]
pub struct Fusion {
    /// The most recent embeddings, as a ring buffer.
    frames: [Embedding; FUSION_FRAMES],
    /// Where the next embedding goes.
    next: usize,
    /// How many of `frames` hold an embedding, at most [`FUSION_FRAMES`].
    count: usize,
}

impl Fusion {
    /// A fusion with no frames yet.
    pub fn new() -> Self {
        Self {
            frames: [Embedding::ZERO; FUSION_FRAMES],
            next: 0,
            count: 0,
        }
    }

    /// Add one embedding, replacing the oldest once the buffer is full.
    pub fn push(&mut self, e: Embedding) {
        self.frames[self.next] = e;
        self.next = (self.next + 1) % FUSION_FRAMES;
        self.count = (self.count + 1).min(FUSION_FRAMES);
    }

    /// Whether [`FUSION_FRAMES`] embeddings have arrived.
    pub fn is_ready(&self) -> bool {
        self.count == FUSION_FRAMES
    }

    /// The normalized average of the last [`FUSION_FRAMES`] embeddings, or
    /// `None` until that many have arrived.
    ///
    /// Embeddings that point in opposite directions cancel. In the extreme
    /// the average is zero, and the result is [`Embedding::ZERO`], which
    /// matches nobody. That is the right answer: the frames disagreed.
    pub fn fused(&self) -> Option<Embedding> {
        if !self.is_ready() {
            return None;
        }
        let mut sum = [0.0f32; EMBEDDING_LEN];
        for frame in &self.frames {
            for (total, value) in sum.iter_mut().zip(frame.values()) {
                *total += value;
            }
        }
        // `from_raw` divides by the length, so the division by
        // `FUSION_FRAMES` that makes it an average is not needed.
        Some(Embedding::from_raw(&sum))
    }

    /// Throw the frames away, for example when the face left the picture.
    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

impl Default for Fusion {
    fn default() -> Self {
        Self::new()
    }
}

/// Hysteresis over the last [`VOTE_WINDOW`] decisions, so the name on the
/// screen does not flicker.
///
/// A decision is a small key: `Some(index)` is the person at that index of
/// [`Gallery::people`], `None` means unknown. The vote remembers the last
/// [`VOTE_WINDOW`] decisions, holds one value, and only replaces it when the
/// [`VOTE_AGREEMENT`] *newest* remembered decisions all agree on a different
/// one.
///
/// The agreeing decisions have to be the newest ones, not just any
/// [`VOTE_AGREEMENT`] of the window. Counting anywhere in the window looks
/// equivalent but is not: a face that the recognizer reads as two different
/// people on alternate frames fills the window with `A B A B A`, where `A`
/// reaches three of five, and one frame later `B` does. The held value would
/// then flip on every single frame, which is the flicker this type exists to
/// stop. Requiring a run of the newest frames means an undecided face keeps
/// whatever was shown before. The price is that one bad frame restarts the
/// run, so a change takes a few frames longer.
///
/// Two things are easy to confuse, so they are kept apart: the held value
/// is `Option<u8>`, where `None` means "unknown", while
/// [`has_decided`](Self::has_decided) says whether anything has been
/// decided at all. A fresh vote holds "unknown" but has not decided, so the
/// first stable "unknown" is still reported as a change.
#[derive(Clone, Copy, Debug)]
pub struct Vote {
    /// The most recent decisions, as a ring buffer.
    window: [Option<u8>; VOTE_WINDOW],
    /// Where the next decision goes.
    next: usize,
    /// How many of `window` hold a decision, at most [`VOTE_WINDOW`].
    count: usize,
    /// The value currently held; `None` means unknown.
    stable: Option<u8>,
    /// Whether `stable` has ever been reported. See the type's
    /// documentation.
    decided: bool,
}

impl Vote {
    /// A vote with no decisions yet, holding "unknown".
    pub fn new() -> Self {
        Self {
            window: [None; VOTE_WINDOW],
            next: 0,
            count: 0,
            stable: None,
            decided: false,
        }
    }

    /// Add one decision.
    ///
    /// Returns `Some(value)` when the vote has now settled on a value that
    /// differs from the one held, that is, when the identity on the screen
    /// should change. Returns `None` while the remembered decisions
    /// disagree, and also while they keep agreeing on the value already
    /// held: a change is reported once, not on every frame.
    pub fn push(&mut self, decision: Option<u8>) -> Option<Option<u8>> {
        self.window[self.next] = decision;
        self.next = (self.next + 1) % VOTE_WINDOW;
        self.count = (self.count + 1).min(VOTE_WINDOW);

        if self.agreement() < VOTE_AGREEMENT {
            return None;
        }
        if self.decided && decision == self.stable {
            return None;
        }
        self.stable = decision;
        self.decided = true;
        Some(decision)
    }

    /// How many of the newest remembered decisions agree with the newest
    /// one, at most [`VOTE_WINDOW`].
    fn agreement(&self) -> usize {
        let newest = self.window[(self.next + VOTE_WINDOW - 1) % VOTE_WINDOW];
        (0..self.count)
            .take_while(|age| {
                self.window[(self.next + VOTE_WINDOW - 1 - age) % VOTE_WINDOW] == newest
            })
            .count()
    }

    /// The value currently held: the index of a person, or `None` for
    /// unknown. A fresh vote holds `None`.
    pub fn stable(&self) -> Option<u8> {
        self.stable
    }

    /// Whether the vote has ever settled on a value. Until it has, the
    /// `None` from [`stable`](Self::stable) is "nothing decided yet" rather
    /// than a decided "unknown".
    pub fn has_decided(&self) -> bool {
        self.decided
    }

    /// Forget the decisions and go back to an undecided "unknown".
    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

impl Default for Vote {
    fn default() -> Self {
        Self::new()
    }
}
