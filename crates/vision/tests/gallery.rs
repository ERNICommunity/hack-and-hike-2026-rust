//! The face-matching decision logic, on synthetic embeddings.
//!
//! No model runs here. The embeddings come from a small random generator,
//! so every score is reproducible and the tests say exactly what they mean:
//! two seeds give two "different people", and `nearby` turns one embedding
//! into another one at a known angle, that is, the "same person" in a
//! slightly different pose.

use hack_and_hike_vision::gallery::{
    Decider, EMBEDDING_LEN, Embedding, FUSION_FRAMES, Fusion, Gallery, ImpostorBank, MAX_NAME,
    MAX_PEOPLE, MAX_TEMPLATES, Match, SureSteps, Thresholds, VOTE_AGREEMENT, VOTE_WINDOW, Vote,
};

/// How far from 1 a normalized length may be.
const LENGTH_TOLERANCE: f32 = 1e-6;

/// Two independent random 512-vectors score below this in size. Random
/// directions in 512 dimensions are almost orthogonal.
const ORTHOGONAL_LIMIT: f32 = 0.15;

/// A reproducible unit vector for `seed`. The values come from a linear
/// congruential generator, the same one in every run, mapped to -1..1.
fn embedding(seed: u32) -> Embedding {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut raw = [0.0f32; EMBEDDING_LEN];
    for value in &mut raw {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *value = (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
    }
    Embedding::from_raw(&raw)
}

/// An embedding `degrees` away from `a`, in the plane it spans with the
/// vector of seed `other_seed`. Turning towards an almost orthogonal
/// direction keeps the angle, and therefore the similarity, close to
/// `cos(degrees)`.
fn nearby(a: &Embedding, other_seed: u32, degrees: f32) -> Embedding {
    let b = embedding(other_seed);
    let (cos, sin) = (degrees.to_radians().cos(), degrees.to_radians().sin());
    let mut raw = [0.0f32; EMBEDDING_LEN];
    for (index, value) in raw.iter_mut().enumerate() {
        *value = a.values()[index] * cos + b.values()[index] * sin;
    }
    Embedding::from_raw(&raw)
}

/// The length of an embedding, which should always be 1 or 0.
fn length(e: &Embedding) -> f32 {
    e.values().iter().map(|v| v * v).sum::<f32>().sqrt()
}

/// A flat bank of the embeddings with these seeds.
fn bank_values(seeds: &[u32]) -> Vec<f32> {
    let mut values = Vec::new();
    for &seed in seeds {
        values.extend_from_slice(embedding(seed).values());
    }
    values
}

#[test]
fn from_raw_normalizes_and_similarity_is_the_cosine() {
    let e = embedding(1);
    assert!(
        (length(&e) - 1.0).abs() < LENGTH_TOLERANCE,
        "length {}",
        length(&e)
    );

    // A vector of large values is scaled down to length 1 as well.
    let big = Embedding::from_raw(&[1000.0; EMBEDDING_LEN]);
    assert!((length(&big) - 1.0).abs() < LENGTH_TOLERANCE);

    // A vector with no direction keeps none.
    let zero = Embedding::from_raw(&[0.0; EMBEDDING_LEN]);
    assert_eq!(length(&zero), 0.0);
    assert_eq!(zero.similarity(&e), 0.0);
    assert_eq!(Embedding::ZERO.values(), zero.values());

    assert!((e.similarity(&e) - 1.0).abs() < 1e-5);
    let mut negated = [0.0f32; EMBEDDING_LEN];
    for (slot, value) in negated.iter_mut().zip(e.values()) {
        *slot = -value;
    }
    let opposite = Embedding::from_raw(&negated);
    assert!((e.similarity(&opposite) + 1.0).abs() < 1e-5);
}

#[test]
fn independent_embeddings_are_almost_orthogonal() {
    for seed in 1..8u32 {
        let (a, b) = (embedding(seed), embedding(seed + 100));
        let score = a.similarity(&b);
        assert!(
            score.abs() < ORTHOGONAL_LIMIT,
            "seeds {seed} and {} scored {score}",
            seed + 100
        );
    }
    // A small turn keeps the faces similar: this is the "same person".
    let a = embedding(1);
    let score = a.similarity(&nearby(&a, 500, 30.0));
    assert!((0.8..0.9).contains(&score), "30 degrees scored {score}");
}

#[test]
fn a_person_holds_up_to_the_template_limit() {
    let mut gallery = Gallery::new();
    let person = gallery.enroll("Alexander").expect("a free slot");
    assert_eq!(person.name(), "Alexander");
    assert!(person.templates().is_empty());
    assert!(!person.is_full());

    // No templates: nothing can match.
    let probe = embedding(1);
    assert_eq!(person.best_similarity(&probe), -1.0);

    for seed in 0..MAX_TEMPLATES as u32 {
        assert!(person.add_template(nearby(&probe, 900 + seed, 40.0)));
    }
    assert!(person.is_full());
    assert_eq!(person.templates().len(), MAX_TEMPLATES);
    assert!(
        !person.add_template(probe),
        "a full person must refuse a template"
    );
    assert_eq!(person.templates().len(), MAX_TEMPLATES);

    // `best_similarity` is the maximum over the templates, so adding one
    // template that is nearly the probe raises it to almost 1.
    let mut gallery = Gallery::new();
    let person = gallery.enroll("Bea").expect("a free slot");
    person.add_template(embedding(2));
    person.add_template(nearby(&probe, 501, 10.0));
    person.add_template(embedding(3));
    let best = person.best_similarity(&probe);
    let each: Vec<f32> = person
        .templates()
        .iter()
        .map(|t| probe.similarity(t))
        .collect();
    let max = each.iter().copied().fold(f32::MIN, f32::max);
    assert_eq!(best, max, "scores {each:?}");
    assert!(best > 0.98, "the 10 degree template scored {best}");
}

#[test]
fn an_impostor_bank_reports_its_closest_member() {
    let probe = embedding(1);

    let empty = ImpostorBank::new(&[]);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.best_similarity(&probe), -1.0);

    // Three unrelated people, and one at 20 degrees from the probe.
    let mut values = bank_values(&[10, 11, 12]);
    values.extend_from_slice(nearby(&probe, 502, 20.0).values());
    let bank = ImpostorBank::new(&values);
    assert_eq!(bank.len(), 4);
    assert!(!bank.is_empty());

    let expected = values
        .chunks_exact(EMBEDDING_LEN)
        .map(|member| {
            probe
                .values()
                .iter()
                .zip(member)
                .map(|(a, b)| a * b)
                .sum::<f32>()
        })
        .fold(f32::MIN, f32::max);
    assert_eq!(bank.best_similarity(&probe), expected);
    assert!(
        bank.best_similarity(&probe) > 0.9,
        "the closest member scored {}",
        bank.best_similarity(&probe)
    );
}

#[test]
#[should_panic(expected = "whole embeddings")]
fn an_impostor_bank_refuses_a_partial_embedding() {
    ImpostorBank::new(&[0.0; EMBEDDING_LEN + 1]);
}

#[test]
fn a_gallery_enrolls_finds_and_forgets_people() {
    let mut gallery = Gallery::new();
    assert!(gallery.is_empty());
    assert_eq!(gallery.len(), 0);
    assert!(gallery.person("Alexander").is_none());

    for index in 0..MAX_PEOPLE {
        let name = ["Alexander", "Bea", "Cem", "Dana"][index];
        gallery
            .enroll(name)
            .expect("a free slot")
            .add_template(embedding(index as u32 + 1));
        assert_eq!(gallery.len(), index + 1);
    }
    assert!(!gallery.is_empty());
    assert_eq!(
        gallery
            .people()
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>(),
        ["Alexander", "Bea", "Cem", "Dana"]
    );

    // Full, duplicated and over-long names are refused.
    assert!(gallery.enroll("Eve").is_none(), "the gallery is full");
    gallery.forget("Cem");
    assert!(gallery.enroll("Bea").is_none(), "a duplicate name");
    assert!(gallery.enroll("").is_none(), "an empty name");
    let too_long = "x".repeat(MAX_NAME + 1);
    assert!(gallery.enroll(&too_long).is_none(), "a long name");
    let longest = "y".repeat(MAX_NAME);
    assert_eq!(
        gallery.enroll(&longest).expect("the free slot").name(),
        longest
    );

    // `forget` closes the gap and reports whether it found anybody.
    assert!(!gallery.forget("Cem"), "Cem is already gone");
    assert!(gallery.forget("Alexander"));
    assert_eq!(gallery.len(), 3);
    assert!(gallery.person("Alexander").is_none());
    assert_eq!(
        gallery
            .people()
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>(),
        ["Bea", "Dana", &longest]
    );

    // `person_mut` reaches the templates of an enrolled person.
    let probe = embedding(7);
    gallery
        .person_mut("Bea")
        .expect("Bea is enrolled")
        .add_template(probe);
    assert_eq!(
        gallery
            .person("Bea")
            .expect("Bea is enrolled")
            .templates()
            .len(),
        2
    );
    assert!(gallery.person("Bea").expect("Bea").best_similarity(&probe) > 0.99);
}

/// A gallery with one person whose templates sit around `probe`, plus the
/// probe itself. The nearest template is 20 degrees away, about 0.94.
fn one_person_gallery() -> (Gallery, Embedding) {
    let probe = embedding(1);
    let mut gallery = Gallery::new();
    let person = gallery.enroll("Alexander").expect("a free slot");
    for (index, degrees) in [20.0f32, 35.0, 50.0].iter().enumerate() {
        person.add_template(nearby(&probe, 600 + index as u32, *degrees));
    }
    (gallery, probe)
}

#[test]
fn a_probe_near_a_template_is_recognized() {
    let (gallery, probe) = one_person_gallery();
    let values = bank_values(&[20, 21, 22, 23]);
    let bank = ImpostorBank::new(&values);
    let outcome = gallery.match_probe(&probe, &bank, &Thresholds::DEFAULT);
    assert!(outcome.is_known(), "score {}", outcome.score());
    assert_eq!(outcome.name(), Some("Alexander"));
    let Match::Known { score, margin, .. } = outcome else {
        panic!("expected a known face");
    };
    assert!(score > 0.9, "score {score}");
    assert!(margin > 0.7, "margin {margin}");
}

#[test]
fn a_closer_impostor_makes_the_margin_fail() {
    let (gallery, probe) = one_person_gallery();

    // The bank holds someone at 10 degrees from the probe, closer than the
    // enrolled person's best template at 20 degrees. The score alone still
    // passes `accept`, so only the margin rule can refuse this.
    let mut values = bank_values(&[20, 21]);
    values.extend_from_slice(nearby(&probe, 601, 10.0).values());
    let bank = ImpostorBank::new(&values);

    let outcome = gallery.match_probe(&probe, &bank, &Thresholds::DEFAULT);
    let Match::Unknown { best_score, margin } = outcome else {
        panic!("expected an unknown face, got {:?}", outcome.name());
    };
    assert!(
        best_score >= Thresholds::DEFAULT.accept,
        "the score itself passed: {best_score}"
    );
    assert!(
        margin < Thresholds::DEFAULT.margin,
        "the margin should fail, it was {margin}"
    );
    assert!(margin < 0.0, "the impostor was closer, margin {margin}");
}

#[test]
fn a_stranger_is_unknown() {
    let (gallery, _) = one_person_gallery();
    let stranger = embedding(4242);
    let values = bank_values(&[20, 21, 22]);
    let bank = ImpostorBank::new(&values);

    let outcome = gallery.match_probe(&stranger, &bank, &Thresholds::DEFAULT);
    assert!(!outcome.is_known());
    assert_eq!(outcome.name(), None);
    assert!(
        outcome.score() < Thresholds::DEFAULT.accept,
        "a stranger scored {}",
        outcome.score()
    );

    // An empty gallery has nobody to offer.
    let empty = Gallery::new();
    let outcome = empty.match_probe(&stranger, &bank, &Thresholds::DEFAULT);
    assert_eq!(outcome.score(), -1.0);
    assert!(!outcome.is_known());
}

#[test]
fn the_better_of_two_people_wins() {
    let probe = embedding(1);
    let mut gallery = Gallery::new();
    // Bea is 15 degrees away, Alexander 40: both pass, Bea wins.
    gallery
        .enroll("Alexander")
        .expect("a free slot")
        .add_template(nearby(&probe, 700, 40.0));
    gallery
        .enroll("Bea")
        .expect("a free slot")
        .add_template(nearby(&probe, 701, 15.0));
    let bank = ImpostorBank::new(&[]);

    let outcome = gallery.match_probe(&probe, &bank, &Thresholds::DEFAULT);
    assert_eq!(outcome.name(), Some("Bea"), "score {}", outcome.score());

    // The order of enrollment does not decide it.
    assert!(gallery.forget("Bea"));
    gallery
        .enroll("Bea")
        .expect("the free slot")
        .add_template(nearby(&probe, 701, 15.0));
    assert_eq!(
        gallery
            .match_probe(&probe, &bank, &Thresholds::DEFAULT)
            .name(),
        Some("Bea")
    );
}

#[test]
fn a_score_below_accept_is_unknown_whatever_the_margin() {
    // The only template is 70 degrees away, about 0.34, and the bank is
    // empty so the margin is a huge 1.34. The `accept` test must still
    // refuse the face.
    let probe = embedding(1);
    let mut gallery = Gallery::new();
    gallery
        .enroll("Alexander")
        .expect("a free slot")
        .add_template(nearby(&probe, 800, 70.0));
    let bank = ImpostorBank::new(&[]);

    let outcome = gallery.match_probe(&probe, &bank, &Thresholds::DEFAULT);
    let Match::Unknown { best_score, margin } = outcome else {
        panic!("expected an unknown face");
    };
    assert!(
        best_score < Thresholds::DEFAULT.accept && best_score > 0.3,
        "score {best_score}"
    );
    assert!(margin > 1.0, "margin {margin}");

    // Lowering `accept` below the score accepts the same face.
    let generous = Thresholds {
        accept: 0.3,
        margin: 0.1,
    };
    assert!(gallery.match_probe(&probe, &bank, &generous).is_known());
    assert_eq!(Thresholds::default(), Thresholds::DEFAULT);
}

#[test]
fn fusion_averages_the_last_frames() {
    let mut fusion = Fusion::new();
    assert!(!fusion.is_ready());
    assert!(fusion.fused().is_none());

    let base = embedding(1);
    let frames: Vec<Embedding> = (0..FUSION_FRAMES)
        .map(|index| nearby(&base, 1000 + index as u32, 5.0))
        .collect();
    for (index, frame) in frames.iter().enumerate() {
        assert!(!fusion.is_ready(), "ready after {index} frames");
        assert!(fusion.fused().is_none());
        fusion.push(*frame);
    }
    assert!(fusion.is_ready());
    let fused = fusion.fused().expect("three frames are in");
    assert!(
        (length(&fused) - 1.0).abs() < LENGTH_TOLERANCE,
        "the average is re-normalized, length {}",
        length(&fused)
    );
    for (index, frame) in frames.iter().enumerate() {
        let score = fused.similarity(frame);
        assert!(score > 0.99, "frame {index} scored {score}");
    }

    // Frames that cancel leave no direction, and that must not panic.
    let mut negated = [0.0f32; EMBEDDING_LEN];
    for (slot, value) in negated.iter_mut().zip(base.values()) {
        *slot = -value;
    }
    let mut fusion = Fusion::new();
    fusion.push(base);
    fusion.push(Embedding::from_raw(&negated));
    fusion.push(embedding(9));
    let fused = fusion.fused().expect("three frames are in");
    assert!(
        fused.similarity(&embedding(9)) > 0.99,
        "only frame 3 is left"
    );

    fusion.clear();
    assert!(!fusion.is_ready());
    assert!(fusion.fused().is_none());
}

#[test]
fn a_vote_only_reports_a_settled_change() {
    let mut vote = Vote::new();
    assert_eq!(vote.stable(), None);
    assert!(!vote.has_decided(), "a fresh vote has decided nothing");

    // One frame of Alexander among unknowns is not enough.
    assert_eq!(vote.push(Some(0)), None);
    assert!(!vote.has_decided());
    for _ in 1..VOTE_AGREEMENT {
        assert_eq!(vote.push(None), None);
    }
    // The window now agrees on "unknown": the first decision ever, so it
    // counts as a change even though the held value was already `None`.
    assert_eq!(vote.push(None), Some(None));
    assert!(vote.has_decided());
    assert_eq!(vote.stable(), None);

    // Agreement that repeats is reported once, not on every frame.
    assert_eq!(vote.push(None), None);
    assert_eq!(vote.push(None), None);

    // Enough frames of Alexander change the identity, once.
    for index in 0..VOTE_AGREEMENT - 1 {
        assert_eq!(vote.push(Some(0)), None, "after {index} frames");
    }
    assert_eq!(vote.push(Some(0)), Some(Some(0)));
    assert_eq!(vote.stable(), Some(0));
    assert_eq!(vote.push(Some(0)), None, "no repeated report");

    // A single stray frame of somebody else changes nothing.
    assert_eq!(vote.push(Some(1)), None);
    assert_eq!(vote.stable(), Some(0));

    vote.clear();
    assert_eq!(vote.stable(), None);
    assert!(!vote.has_decided());
}

#[test]
fn an_undecided_vote_never_settles() {
    let mut vote = Vote::new();
    // A face read as two different people on alternate frames fills the
    // window with `A B A B A`, where A holds three of the five slots. A
    // plain count would settle on A here, and on B one frame later, so the
    // name would flip on every frame. Only the newest frames count, so the
    // vote stays undecided instead.
    for index in 0..4 * VOTE_WINDOW {
        let decision = Some((index % 2) as u8);
        assert_eq!(vote.push(decision), None, "frame {index}");
    }
    assert!(!vote.has_decided());
    assert_eq!(vote.stable(), None);

    // The same holds once a value is held: alternation never takes it away.
    for _ in 0..VOTE_AGREEMENT {
        vote.push(Some(0));
    }
    assert_eq!(vote.stable(), Some(0));
    for index in 0..4 * VOTE_WINDOW {
        let decision = Some((index % 2) as u8 + 1);
        assert_eq!(vote.push(decision), None, "frame {index}");
    }
    assert_eq!(vote.stable(), Some(0), "the held value survives noise");
}

#[test]
fn fusion_averages_what_has_arrived_so_far() {
    let mut fusion = Fusion::new();
    assert_eq!(fusion.frames(), 0);
    assert!(fusion.fused_so_far().is_none());

    let base = embedding(1);
    fusion.push(base);
    assert_eq!(fusion.frames(), 1);
    let one = fusion.fused_so_far().expect("one frame is in");
    assert!(one.similarity(&base) > 0.9999, "one frame is itself");

    // Two frames 60 degrees to either side of nothing in particular: the
    // average is nearer to the base than either of them.
    let mut fusion = Fusion::new();
    let frames = [nearby(&base, 1000, 60.0), nearby(&base, 1001, 60.0)];
    fusion.push(frames[0]);
    fusion.push(frames[1]);
    assert_eq!(fusion.frames(), 2);
    let two = fusion.fused_so_far().expect("two frames are in");
    assert!((length(&two) - 1.0).abs() < LENGTH_TOLERANCE);
    assert!(two.similarity(&base) > frames[0].similarity(&base) + 0.05);

    // With every frame in, it is the full average.
    fusion.push(nearby(&base, 1002, 60.0));
    assert_eq!(fusion.frames(), FUSION_FRAMES);
    let all = fusion.fused_so_far().expect("three frames are in");
    let full = fusion.fused().expect("three frames are in");
    assert_eq!(all.values(), full.values());
}

#[test]
fn a_settled_vote_holds_until_the_decisions_agree_again() {
    let mut vote = Vote::new();
    assert!(vote.settle(Some(0)), "the first value is a change");
    assert!(vote.has_decided());
    assert_eq!(vote.stable(), Some(0));
    assert!(!vote.settle(Some(0)), "the same value again is none");

    // Settling forgets the decisions before it: the run starts again.
    assert_eq!(vote.push(None), None);
    assert_eq!(vote.push(None), None);
    assert!(!vote.settle(Some(0)));
    for index in 0..VOTE_AGREEMENT - 1 {
        assert_eq!(vote.push(None), None, "after {index} decisions");
    }
    assert_eq!(vote.push(None), Some(None));
    assert!(vote.settle(Some(1)), "another value is a change");
}

/// A gallery with one person whose only template is `embedding(1)`, and
/// a bank of strangers. A probe `degrees` away from the template scores
/// `cos(degrees)`.
fn decider_setup() -> (Gallery, Vec<f32>) {
    let mut gallery = Gallery::new();
    let person = gallery.enroll("Alexander").expect("a free slot");
    person.add_template(embedding(1));
    (gallery, bank_values(&[20, 21, 22, 23]))
}

#[test]
fn a_sure_face_is_named_on_its_first_frame() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    assert_eq!(decider.shown(), None);

    // 40 degrees: 0.77, above the limit of one frame.
    let probe = nearby(&embedding(1), 700, 40.0);
    let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
    assert!(
        decision.score > sure.limit(thresholds.accept, 1),
        "score {}",
        decision.score
    );
    assert_eq!(decision.frames, 1);
    assert!(decision.sure);
    assert_eq!(decision.verdict, Some(Some(0)));
    assert_eq!(decision.shown, Some(Some(0)));
    assert_eq!(decider.shown(), Some(Some(0)));

    // The same face again changes nothing on the screen.
    let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
    assert_eq!(decision.frames, 2);
    assert!(decision.sure);
    assert_eq!(decision.shown, None);
    assert_eq!(decider.shown(), Some(Some(0)));
}

#[test]
fn a_face_just_above_the_limit_waits_for_the_vote() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();

    // 66 degrees: 0.41, above the accept limit and below every sure
    // limit. The same frame every time, so the average is that frame.
    let probe = nearby(&embedding(1), 700, 66.0);
    let score = probe.similarity(&embedding(1));
    assert!(score > thresholds.accept && score < sure.limit(thresholds.accept, FUSION_FRAMES));
    let needed = FUSION_FRAMES + VOTE_AGREEMENT - 1;
    for frame in 1..=needed {
        let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
        assert!(!decision.sure, "frame {frame}");
        let expected = (frame >= FUSION_FRAMES).then_some(Some(0));
        assert_eq!(decision.verdict, expected, "frame {frame}");
        let shown = (frame == needed).then_some(Some(0));
        assert_eq!(decision.shown, shown, "frame {frame}");
    }
    assert_eq!(decider.shown(), Some(Some(0)));
}

#[test]
fn without_sure_steps_every_face_waits_for_the_vote() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let mut decider = Decider::new();
    let needed = FUSION_FRAMES + VOTE_AGREEMENT - 1;
    for frame in 1..=needed {
        // The template itself: a score of 1.
        let decision = decider.push(
            embedding(1),
            &gallery,
            &bank,
            &Thresholds::DEFAULT,
            &SureSteps::NEVER,
        );
        assert!(!decision.sure);
        let shown = (frame == needed).then_some(Some(0));
        assert_eq!(decision.shown, shown, "frame {frame}");
    }
}

#[test]
fn a_stranger_is_unknown_after_the_first_average() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    for frame in 1..=2 * FUSION_FRAMES {
        let stranger = embedding(900 + frame as u32);
        let decision = decider.push(stranger, &gallery, &bank, &thresholds, &sure);
        assert!(!decision.sure);
        assert!(decision.score < thresholds.accept);
        let verdict = (frame >= FUSION_FRAMES).then_some(None);
        assert_eq!(decision.verdict, verdict, "frame {frame}");
        // Shown once, with the first average, and not again.
        let shown = (frame == FUSION_FRAMES).then_some(None);
        assert_eq!(decision.shown, shown, "frame {frame}");
    }
    assert_eq!(decider.shown(), Some(None));

    // The enrolled person steps in after a pause: sure on the first
    // frame, so named at once over the "unknown", without the vote.
    decider.pause();
    let decision = decider.push(embedding(1), &gallery, &bank, &thresholds, &sure);
    assert_eq!(decision.shown, Some(Some(0)));
}

#[test]
fn a_name_gives_way_to_unknown_only_by_the_vote() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    let decision = decider.push(embedding(1), &gallery, &bank, &thresholds, &sure);
    assert_eq!(decision.shown, Some(Some(0)));

    // The face is not usable for a moment: the name stays.
    decider.pause();
    assert_eq!(decider.shown(), Some(Some(0)));

    // A stranger takes the place: unknown after the first average and
    // the decisions that must agree.
    let needed = FUSION_FRAMES + VOTE_AGREEMENT - 1;
    for frame in 1..=needed {
        let stranger = embedding(900 + frame as u32);
        let decision = decider.push(stranger, &gallery, &bank, &thresholds, &sure);
        let shown = (frame == needed).then_some(None);
        assert_eq!(decision.shown, shown, "frame {frame}");
    }
    assert_eq!(decider.shown(), Some(None));

    decider.clear();
    assert_eq!(decider.shown(), None);
}

#[test]
fn the_sure_limits_never_rise_with_the_frames() {
    // More frames averaged never make a sure name harder to reach. (With
    // MFN_S8_V1 the calibrated limits are the same for every count.)
    let sure = SureSteps::DEFAULT;
    let accept = Thresholds::DEFAULT.accept;
    for frames in 1..FUSION_FRAMES {
        assert!(sure.limit(accept, frames) >= sure.limit(accept, frames + 1));
    }
    assert!(sure.limit(accept, FUSION_FRAMES) > accept);
    // More frames than the average holds count as all of them.
    assert_eq!(
        sure.limit(accept, FUSION_FRAMES + 3),
        sure.limit(accept, FUSION_FRAMES)
    );
    assert_eq!(SureSteps::default(), SureSteps::DEFAULT);
    assert!(SureSteps::NEVER.limit(accept, 1).is_infinite());
}

#[test]
fn the_gallery_fits_in_the_firmware_memory() {
    let person = core::mem::size_of::<hack_and_hike_vision::gallery::Person>();
    let gallery = core::mem::size_of::<Gallery>();
    let embedding = core::mem::size_of::<Embedding>();
    println!("size_of::<Embedding>() = {embedding} bytes");
    println!("size_of::<Person>()    = {person} bytes");
    println!("size_of::<Gallery>()   = {gallery} bytes");
    println!(
        "size_of::<Fusion>()    = {} bytes",
        core::mem::size_of::<Fusion>()
    );
    println!(
        "size_of::<Vote>()      = {} bytes",
        core::mem::size_of::<Vote>()
    );

    assert_eq!(embedding, EMBEDDING_LEN * 4);
    assert!(person >= MAX_TEMPLATES * embedding);
    assert!(gallery >= MAX_PEOPLE * person);
    // A gallery has to stay well under the 512 KiB of internal RAM.
    assert!(
        gallery < 128 * 1024,
        "a gallery of {gallery} bytes is too big"
    );
}

#[test]
fn a_sure_face_takes_over_from_another_person() {
    let (mut gallery, values) = decider_setup();
    gallery
        .enroll("Bea")
        .expect("a free slot")
        .add_template(embedding(2));
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    let decision = decider.push(embedding(2), &gallery, &bank, &thresholds, &sure);
    assert_eq!(decision.shown, Some(Some(1)));

    decider.pause();
    let decision = decider.push(embedding(1), &gallery, &bank, &thresholds, &sure);
    assert!(decision.sure);
    assert_eq!(decision.shown, Some(Some(0)));
    assert_eq!(decider.shown(), Some(Some(0)));
}

#[test]
fn a_sure_score_must_beat_the_bank_too() {
    let (gallery, mut values) = decider_setup();
    // A stranger in the bank closer to the probe than the template.
    let probe = nearby(&embedding(1), 700, 40.0);
    values.extend_from_slice(probe.values());
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
    assert!(decision.score > sure.limit(thresholds.accept, 1));
    assert!(!decision.sure);
    assert_eq!(decision.shown, None);
}

#[test]
fn unknown_is_shown_at_once_while_nothing_is_shown() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    let mut decider = Decider::new();
    // Plain decisions for the person, not enough to be shown.
    let probe = nearby(&embedding(1), 700, 66.0);
    for _ in 0..FUSION_FRAMES {
        let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
        assert_eq!(decision.shown, None);
    }
    assert_eq!(decider.shown(), None);

    // A stranger: unknown with its first average.
    decider.pause();
    for frame in 1..=FUSION_FRAMES {
        let stranger = embedding(900 + frame as u32);
        let decision = decider.push(stranger, &gallery, &bank, &thresholds, &sure);
        let shown = (frame == FUSION_FRAMES).then_some(None);
        assert_eq!(decision.shown, shown, "frame {frame}");
    }
}

#[test]
fn the_vote_outlasts_a_pause_but_not_a_clear() {
    let (gallery, values) = decider_setup();
    let bank = ImpostorBank::new(&values);
    let (thresholds, sure) = (Thresholds::DEFAULT, SureSteps::DEFAULT);
    // Just above the limit: plain decisions, shown by the vote only.
    let probe = nearby(&embedding(1), 700, 66.0);
    let almost = FUSION_FRAMES + VOTE_AGREEMENT - 2;
    for forget in [false, true] {
        let mut decider = Decider::new();
        for _ in 0..almost {
            let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
            assert_eq!(decision.shown, None);
        }
        if forget {
            decider.clear();
        } else {
            decider.pause();
        }
        // After a pause the next average adds the one missing decision;
        // after a clear the vote starts over.
        for frame in 1..=FUSION_FRAMES {
            let decision = decider.push(probe, &gallery, &bank, &thresholds, &sure);
            let shown = (!forget && frame == FUSION_FRAMES).then_some(Some(0));
            assert_eq!(decision.shown, shown, "forget {forget}, frame {frame}");
        }
    }
}
