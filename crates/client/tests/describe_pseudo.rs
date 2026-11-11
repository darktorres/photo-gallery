//! The filter line in the pseudo-locale has no English word: every word of it is the bundle's (MODULARITY_REVIEW_4,
//! M-21). A test binary of its own, as the pseudo-locale is a process-wide switch that the other tests' English
//! would see.

use pg_client::text::{describe, set_pseudo};
use pg_model::date::Day;
use pg_model::media::MediaType;
use pg_model::query::{BorderFacet, DateRange, Query, Range, RatingFilter, Shape, Status};

#[test]
fn every_word_of_the_filter_line_is_the_bundles() {
    set_pseudo(true);
    let day = Day::parse("2024-01-02");
    // Every facet but the user's own text (an extension, a name needle), which no locale words.
    let q = Query {
        types: vec![MediaType::Image, MediaType::Video],
        ratings: vec![RatingFilter::Unrated, RatingFilter::try_from(5).unwrap()],
        size_mb: Range { min: Some(1.5), max: Some(20.0) },
        megapixels: Range { min: Some(2.0), max: None },
        shapes: vec![Shape::Landscape, Shape::Square],
        taken: DateRange { from: day, to: day },
        modified: DateRange { from: day, to: None },
        duration_s: Range { min: None, max: Some(30.0) },
        duplicates: Some(true),
        status: vec![Status::Imported, Status::Failed],
        borders: vec![BorderFacet::Black, BorderFacet::Blur],
        border_marked: Some(false),
        ..Query::default()
    };
    let line = describe(&q);
    // The pseudo-locale accents every ASCII letter, so a run of them is English that skipped the bundle.
    let english: Vec<&str> = line.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| w.len() > 1).collect();
    assert!(english.is_empty(), "English in {line}: {english:?}");
}
