//! Enum values travel with the labels the database type stores.
//!
//! `rename_all = "snake_case"` spells `Pph22` as `pph22`, while the Postgres enum and the
//! schema hold `pph_22`: without a per-variant rename, a stored row cannot be read and a
//! client sending the schema's label is refused.

use backbone_payment::{AgingBucketName, WithholdingTaxType};

#[test]
fn a_label_with_a_digit_boundary_round_trips() {
    for (v, label) in [
        (WithholdingTaxType::Pph22, "pph_22"),
        (WithholdingTaxType::Pph23, "pph_23"),
        (WithholdingTaxType::Pph26, "pph_26"),
    ] {
        assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{label}\""));
        assert_eq!(serde_json::from_str::<WithholdingTaxType>(&format!("\"{label}\"")).unwrap(), v);
    }
    for (v, label) in [
        (AgingBucketName::Bucket130, "bucket_1_30"),
        (AgingBucketName::Bucket3160, "bucket_31_60"),
        (AgingBucketName::Bucket6190, "bucket_61_90"),
        (AgingBucketName::Bucket90p, "bucket_90p"),
    ] {
        assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{label}\""));
        assert_eq!(v.to_string(), label, "Display and the wire agree");
    }
}
