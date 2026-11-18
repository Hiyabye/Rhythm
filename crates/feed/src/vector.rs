//! The int8 vectors file published beside a records file (#166 O19a): each
//! vector is a little-endian f32 scale followed by `DIMENSIONS` int8 values,
//! `VECTOR_BYTES` in all, with no header. #170 measured this encoding within
//! the local CPU-vs-Metal baseline.
use crate::{Error, DIMENSIONS, VECTOR_BYTES};

/// Symmetric quantization: the largest magnitude maps to ±127.
pub fn quantize(vector: &[f32]) -> Result<[u8; VECTOR_BYTES], Error> {
    // `f32::max` skips NaN, so finiteness is checked separately.
    let largest = vector.iter().fold(0f32, |max, x| max.max(x.abs()));
    if vector.len() != DIMENSIONS || !largest.is_normal() || !vector.iter().all(|x| x.is_finite()) {
        return Err(Error::Invalid(
            "not a finite nonzero vector of the feed's size".into(),
        ));
    }
    let scale = largest / 127.0;
    let mut bytes = [0u8; VECTOR_BYTES];
    bytes[..4].copy_from_slice(&scale.to_le_bytes());
    for (byte, x) in bytes[4..].iter_mut().zip(vector) {
        *byte = ((x / scale).round().clamp(-127.0, 127.0) as i8) as u8;
    }
    Ok(bytes)
}

pub fn encode(vectors: &[[u8; VECTOR_BYTES]]) -> Vec<u8> {
    vectors.concat()
}

/// Every vector of a file, normalized to unit length.
pub fn decode(bytes: &[u8]) -> Result<Vec<Vec<f32>>, Error> {
    if !bytes.len().is_multiple_of(VECTOR_BYTES) {
        return Err(Error::Invalid("vectors file is not whole vectors".into()));
    }
    bytes
        .as_chunks::<VECTOR_BYTES>()
        .0
        .iter()
        .map(|chunk| {
            let scale = f32::from_le_bytes(chunk[..4].try_into().expect("four bytes"));
            let values: Vec<f32> = chunk[4..].iter().map(|b| f32::from(*b as i8)).collect();
            let norm = values.iter().map(|x| x * x).sum::<f32>().sqrt();
            // The scale cancels out in the normalization, but a vector whose
            // scale is not positive and finite was not written by `quantize`.
            if !scale.is_normal() || scale < 0.0 || !norm.is_normal() {
                return Err(Error::Invalid(
                    "vectors file holds an invalid vector".into(),
                ));
            }
            Ok(values.iter().map(|x| x / norm).collect())
        })
        .collect()
}
