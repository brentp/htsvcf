//! Genotype parsing and manipulation for VCF/BCF records.
//!
//! This module provides types and functions for working with VCF genotypes (GT field).
//! The [`Genotype`] struct represents a parsed genotype with allele indices and phase
//! information.
//!
//! # Example
//!
//! ```no_run
//! use htsvcf_core::genotype::{Genotype, record_genotypes, record_set_genotypes};
//! use htsvcf_core::header::Header;
//! use rust_htslib::bcf::{self, Read};
//!
//! let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
//! let header = unsafe { Header::new(reader.header().inner) };
//!
//! let mut rec = reader.empty_record();
//! reader.read(&mut rec).unwrap();
//!
//! // Read genotypes
//! let gts = record_genotypes(&rec, &header, None);
//! for gt in &gts {
//!     println!("alleles: {:?}, phase: {:?}", gt.alleles, gt.phase);
//! }
//!
//! // Modify and write back
//! let new_gts = vec![
//!     Genotype { alleles: vec![Some(0), Some(1)], phase: vec![false] },
//!     Genotype { alleles: vec![Some(1), Some(1)], phase: vec![true] },
//! ];
//! record_set_genotypes(&mut rec, &new_gts).unwrap();
//! ```

use crate::header::Header;
use rust_htslib::bcf;

/// Represents a parsed genotype for a single sample.
///
/// The `alleles` vector contains allele indices (0 = REF, 1+ = ALT),
/// with `None` representing missing alleles (`.` in VCF notation).
///
/// The `phase` vector indicates the phase separator between consecutive alleles:
/// - `false` = unphased (`/`)
/// - `true` = phased (`|`)
///
/// The length of `phase` is always `alleles.len() - 1` (or 0 for haploid).
///
/// # Examples
///
/// | GT String | alleles | phase |
/// |-----------|---------|-------|
/// | `0/1` | `[Some(0), Some(1)]` | `[false]` |
/// | `1\|1` | `[Some(1), Some(1)]` | `[true]` |
/// | `./1` | `[None, Some(1)]` | `[false]` |
/// | `1` | `[Some(1)]` | `[]` |
/// | `0/1\|2` | `[Some(0), Some(1), Some(2)]` | `[false, true]` |
#[derive(Debug, Clone, PartialEq)]
pub struct Genotype {
    /// Allele indices. `None` represents a missing allele (`.`).
    pub alleles: Vec<Option<i32>>,
    /// Phase separators. `phase[i]` indicates whether `alleles[i+1]` is phased
    /// with `alleles[i]` (`true` = `|`, `false` = `/`).
    /// Length is always `alleles.len() - 1` (or 0 for haploid).
    pub phase: Vec<bool>,
}

/// Parse genotypes for all samples or a subset of samples.
///
/// Returns a vector of [`Genotype`] structs, one per requested sample.
/// If `subset` is `None`, returns genotypes for all samples in header order.
/// If `subset` is `Some(names)`, returns genotypes only for those samples
/// in the order specified (unknown sample names are skipped).
///
/// Returns an empty vector if:
/// - The record has no GT field
/// - The record has no samples
/// - None of the requested samples exist
pub fn record_genotypes(
    record: &bcf::Record,
    header: &Header,
    subset: Option<&[&str]>,
) -> Vec<Genotype> {
    let sample_count = record.sample_count() as usize;
    if sample_count == 0 {
        return Vec::new();
    }

    let gts = match record.genotypes() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    // Determine which sample indices to include
    let sample_indices: Vec<usize> = match subset {
        None => (0..sample_count).collect(),
        Some(names) => {
            let name_to_idx = header.sample_name_to_idx();
            names
                .iter()
                .filter_map(|name| name_to_idx.get(*name).copied())
                .collect()
        }
    };

    sample_indices
        .iter()
        .map(|&idx| parse_genotype(&gts.get(idx)))
        .collect()
}

/// Set genotypes for all samples.
///
/// Takes a slice of [`Genotype`] structs (same format returned by [`record_genotypes()`]).
/// The slice is flattened into `GenotypeAllele` values and written via `push_genotypes()`.
///
/// # Arguments
///
/// * `record` - The BCF record to modify
/// * `genotypes` - One `Genotype` per sample, in sample order. The length must
///   exactly match the number of samples in the record.
///
/// # Errors
///
/// Returns an error if:
/// - The genotypes slice length doesn't match the sample count
/// - The GT field cannot be set (e.g., not defined in header)
pub fn record_set_genotypes(
    record: &mut bcf::Record,
    genotypes: &[Genotype],
) -> Result<(), rust_htslib::errors::Error> {
    use rust_htslib::bcf::record::GenotypeAllele;
    use rust_htslib::errors::Error;

    let sample_count = record.sample_count() as usize;
    if genotypes.len() != sample_count {
        return Err(Error::BcfSetTag {
            tag: format!(
                "GT: genotypes length ({}) must match sample count ({})",
                genotypes.len(),
                sample_count
            ),
        });
    }

    let mut alleles: Vec<GenotypeAllele> = Vec::new();

    for gt in genotypes {
        for (i, allele) in gt.alleles.iter().enumerate() {
            // First allele is always unphased; subsequent alleles check phase[i-1]
            let is_phased = if i == 0 {
                false
            } else {
                gt.phase.get(i - 1).copied().unwrap_or(false)
            };

            match (allele, is_phased) {
                (Some(idx), false) => alleles.push(GenotypeAllele::Unphased(*idx)),
                (Some(idx), true) => alleles.push(GenotypeAllele::Phased(*idx)),
                (None, false) => alleles.push(GenotypeAllele::UnphasedMissing),
                (None, true) => alleles.push(GenotypeAllele::PhasedMissing),
            }
        }
    }

    record.push_genotypes(&alleles)?;
    record.unpack();
    Ok(())
}

/// Parse a single genotype from rust-htslib's Genotype type.
pub(crate) fn parse_genotype(gt: &rust_htslib::bcf::record::Genotype) -> Genotype {
    use rust_htslib::bcf::record::GenotypeAllele;

    let mut alleles: Vec<Option<i32>> = Vec::with_capacity(gt.len());
    let mut phase: Vec<bool> = Vec::with_capacity(gt.len().saturating_sub(1));

    for (i, allele) in gt.iter().enumerate() {
        match allele {
            GenotypeAllele::Unphased(idx) => {
                alleles.push(Some(*idx));
                // First allele has no preceding separator, subsequent unphased alleles mean '/'
                if i > 0 {
                    phase.push(false);
                }
            }
            GenotypeAllele::Phased(idx) => {
                alleles.push(Some(*idx));
                // Phased means '|' separator before this allele
                if i > 0 {
                    phase.push(true);
                }
            }
            GenotypeAllele::UnphasedMissing => {
                alleles.push(None);
                if i > 0 {
                    phase.push(false);
                }
            }
            GenotypeAllele::PhasedMissing => {
                alleles.push(None);
                if i > 0 {
                    phase.push(true);
                }
            }
        }
    }

    Genotype { alleles, phase }
}

/// Parse a single sample's genotype by index.
pub(crate) fn parse_genotype_for_sample(
    record: &bcf::Record,
    sample_idx: usize,
) -> Option<Genotype> {
    let gts = record.genotypes().ok()?;
    Some(parse_genotype(&gts.get(sample_idx)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;

    #[test]
    fn test_set_genotypes() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\tS3\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0/1\t1|1\t./.\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("genotype-set.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();

        // Original genotypes
        let orig = record_genotypes(&rec, &header, None);
        assert_eq!(orig.len(), 3);
        assert_eq!(orig[0].alleles, vec![Some(0), Some(1)]);
        assert_eq!(orig[0].phase, vec![false]);
        assert_eq!(orig[1].alleles, vec![Some(1), Some(1)]);
        assert_eq!(orig[1].phase, vec![true]);
        assert_eq!(orig[2].alleles, vec![None, None]);

        // Set new genotypes
        let new_gts = vec![
            Genotype {
                alleles: vec![Some(1), Some(0)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![Some(0), Some(0)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![Some(1), Some(1)],
                phase: vec![true],
            },
        ];
        record_set_genotypes(&mut rec, &new_gts).unwrap();

        // Verify
        let updated = record_genotypes(&rec, &header, None);
        assert_eq!(updated.len(), 3);
        assert_eq!(updated[0].alleles, vec![Some(1), Some(0)]);
        assert_eq!(updated[0].phase, vec![false]);
        assert_eq!(updated[1].alleles, vec![Some(0), Some(0)]);
        assert_eq!(updated[1].phase, vec![false]);
        assert_eq!(updated[2].alleles, vec![Some(1), Some(1)]);
        assert_eq!(updated[2].phase, vec![true]);

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_genotypes_with_missing() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0/1\t1/1\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("genotype-missing.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();

        // Set genotypes with missing alleles
        let new_gts = vec![
            Genotype {
                alleles: vec![None, Some(1)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![None, Some(0)],
                phase: vec![true],
            },
        ];
        record_set_genotypes(&mut rec, &new_gts).unwrap();

        let updated = record_genotypes(&rec, &header, None);
        assert_eq!(updated[0].alleles, vec![None, Some(1)]);
        assert_eq!(updated[0].phase, vec![false]);
        assert_eq!(updated[1].alleles, vec![None, Some(0)]);
        assert_eq!(updated[1].phase, vec![true]);

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_genotypes_haploid() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0\t1\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("genotype-haploid.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();

        // Set haploid genotypes
        let new_gts = vec![
            Genotype {
                alleles: vec![Some(1)],
                phase: vec![],
            },
            Genotype {
                alleles: vec![Some(0)],
                phase: vec![],
            },
        ];
        record_set_genotypes(&mut rec, &new_gts).unwrap();

        let updated = record_genotypes(&rec, &header, None);
        assert_eq!(updated[0].alleles, vec![Some(1)]);
        assert_eq!(updated[0].phase.len(), 0);
        assert_eq!(updated[1].alleles, vec![Some(0)]);
        assert_eq!(updated[1].phase.len(), 0);

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_genotypes_wrong_count_errors() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\tS3\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0/1\t1|1\t./.\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("genotype-wrong-count.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();

        // Too few genotypes (2 for 3 samples)
        let too_few = vec![
            Genotype {
                alleles: vec![Some(0), Some(1)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![Some(1), Some(1)],
                phase: vec![true],
            },
        ];
        let result = record_set_genotypes(&mut rec, &too_few);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("2") && err_msg.contains("3"),
            "Error should mention counts: {}",
            err_msg
        );

        // Too many genotypes (4 for 3 samples)
        let too_many = vec![
            Genotype {
                alleles: vec![Some(0), Some(1)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![Some(1), Some(1)],
                phase: vec![true],
            },
            Genotype {
                alleles: vec![Some(0), Some(0)],
                phase: vec![false],
            },
            Genotype {
                alleles: vec![Some(1), Some(0)],
                phase: vec![false],
            },
        ];
        let result = record_set_genotypes(&mut rec, &too_many);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("4") && err_msg.contains("3"),
            "Error should mention counts: {}",
            err_msg
        );

        let _ = std::fs::remove_file(&vcf_path);
    }
}
