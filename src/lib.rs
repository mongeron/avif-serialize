mod boxes;
mod writer;

use crate::boxes::*;
use arrayvec::ArrayVec;
use std::io;

/// Makes an AVIF file given encoded AV1 data (create the data with [`rav1e`](//lib.rs/rav1e))
///
/// `color_av1_data` is already-encoded AV1 image data for the color channels (YUV, RGB, etc.).
/// The color image MUST have been encoded without chroma subsampling AKA YUV444 (`Cs444` in `rav1e`)
/// AV1 handles full-res color so effortlessly, you should never need chroma subsampling ever again.
///
/// Optional `alpha_av1_data` is a monochrome image (`rav1e` calls it "YUV400"/`Cs400`) representing transparency.
/// Alpha adds a lot of header bloat, so don't specify it unless it's necessary.
///
/// `width`/`height` is image size in pixels. It must of course match the size of encoded image data.
/// `depth_bits` should be 8, 10 or 12, depending on how the image was encoded (typically 8).
///
/// Color and alpha must have the same dimensions and depth.
///
/// Data is written (streamed) to `into_output`.
pub fn serialize<W: io::Write>(into_output: W, color_av1_data: &[u8], alpha_av1_data: Option<&[u8]>, width: u32, height: u32, depth_bits: u8) -> io::Result<()> {
    let mut image_items = ArrayVec::new();
    let mut iloc_items = ArrayVec::new();
    let mut av1c_items = ArrayVec::new();
    let mut compatible_brands = ArrayVec::new();
    let mut ipma_entries = ArrayVec::new();
    let mut data_chunks = ArrayVec::<[&[u8]; 4]>::new();
    let mut iref = None;
    let mut auxc = None;
    let color_image_id = 1;
    let alpha_image_id = 2;
    let high_bitdepth = depth_bits >= 10;
    let twelve_bit = depth_bits >= 12;

    image_items.push(InfeBox {
        id: color_image_id,
        typ: FourCC(*b"av01"),
        name: "",
    });
    // This is redundant, but Chrome wants it, and checks that it matches :(
    av1c_items.push(Av1CBox {
        seq_profile: false,
        seq_level_idx_0: 0,
        seq_tier_0: false,
        high_bitdepth,
        twelve_bit,
        monochrome: false,
        chroma_subsampling_x: false,
        chroma_subsampling_y: false,
        chroma_sample_position: 0,
    });
    ipma_entries.push(IpmaEntry {
        item_id: color_image_id,
        prop_ids: [1, 2].iter().copied().collect(),
    });

    if let Some(alpha_data) = alpha_av1_data {
        image_items.push(InfeBox {
            id: alpha_image_id,
            typ: FourCC(*b"av01"),
            name: "",
        });
        av1c_items.push(Av1CBox {
            seq_profile: false,
            seq_level_idx_0: 0,
            seq_tier_0: false,
            high_bitdepth,
            twelve_bit,
            monochrome: true,
            chroma_subsampling_x: false,
            chroma_subsampling_y: false,
            chroma_sample_position: 0,
        });
        // that's a silly way to add 1 bit of information, isn't it?
        auxc = Some(AuxCBox {
            urn: "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha",
        });
        iref = Some(IrefBox {
            entry: IrefEntryBox {
                from_id: alpha_image_id,
                to_id: color_image_id,
                typ: FourCC(*b"auxl"),
            },
        });
        ipma_entries.push(IpmaEntry {
            item_id: alpha_image_id,
            prop_ids: [3, 4].iter().copied().collect(),
        });

        // Use interleaved color and alpha, with alpha first.
        // Makes it possible to display partial image.
        let (c1, c2) = color_av1_data.split_at(color_av1_data.len() / 2);
        let (a1, a2) = alpha_data.split_at(alpha_data.len() / 2);
        iloc_items.push(IlocItem {
            id: color_image_id,
            extents: [
                IlocExtent {
                    offset: IlocOffset::Relative(a1.len()),
                    len: c1.len(),
                },
                IlocExtent {
                    offset: IlocOffset::Relative(a1.len() + c1.len() + a2.len()),
                    len: c2.len(),
                }
            ].into()
        });
        iloc_items.push(IlocItem {
            id: alpha_image_id,
            extents: [
                IlocExtent {
                    offset: IlocOffset::Relative(0),
                    len: a1.len(),
                },
                IlocExtent {
                    offset: IlocOffset::Relative(a1.len() + c1.len()),
                    len: a2.len(),
                }
            ].into()
        });
        data_chunks.push(a1);
        data_chunks.push(c1);
        data_chunks.push(a2);
        data_chunks.push(c2);
    } else {
        // that's a quirk only for opaque images in Firefox
        compatible_brands.push(FourCC(*b"mif1"));

        let mut extents = ArrayVec::new();
        extents.push(IlocExtent {
            offset: IlocOffset::Relative(0),
            len: color_av1_data.len(),
        });
        iloc_items.push(IlocItem {
            id: color_image_id,
            extents,
        });
        data_chunks.push(color_av1_data);
    }

    let mut boxes = AvifFile {
        ftyp: FtypBox {
            major_brand: FourCC(*b"avif"),
            minor_version: 0,
            compatible_brands,
        },
        meta: MetaBox {
            iinf: IinfBox {
                items: image_items,
            },
            pitm: PitmBox(color_image_id),
            iloc: IlocBox {
                items: iloc_items,
            },
            iprp: IprpBox {
                ipco: IpcoBox {
                    // This is redundant data inherited from the HEIF spec.
                    ispe: IspeBox {
                        width,
                        height,
                    },
                    av1c: av1c_items,
                    auxc,
                },
                // It's not enough to define these properties,
                // they must be assigned to the image
                ipma: IpmaBox {
                    entries: ipma_entries,
                },
            },
            iref,
        },
        // Here's the actual data. If HEIF wasn't such a kitchen sink, this
        // would have been the only data this file needs.
        mdat: MdatBox {
            data_chunks: &data_chunks,
        }
    };

    boxes.write(into_output)
}

/// See [`serialize`] for description. This one makes a `Vec` instead of using `io::Write`.
pub fn serialize_to_vec(color_av1_data: &[u8], alpha_av1_data: Option<&[u8]>, width: u32, height: u32, depth_bits: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(color_av1_data.len() + alpha_av1_data.map_or(0, |a| a.len()) + 400);
    serialize(&mut out, color_av1_data, alpha_av1_data, width, height, depth_bits).unwrap(); // Vec can't fail
    out
}

#[test]
fn test_roundtrip_parse_mp4() {
    let test_img = b"av12356abc";
    let avif = serialize_to_vec(test_img, None, 10, 20, 8);

    let mut ctx = mp4parse::AvifContext::new();
    mp4parse::read_avif(&mut avif.as_slice(), &mut ctx).unwrap();

    assert_eq!(&test_img[..], ctx.primary_item.as_slice());
}

#[test]
fn test_roundtrip_parse_avif() {
    let test_img = [1,2,3,4,5,6];
    let test_alpha = [77,88,99];
    let avif = serialize_to_vec(&test_img, Some(&test_alpha), 10, 20, 8);

    let ctx = avif_parse::read_avif(&mut avif.as_slice()).unwrap();

    assert_eq!(&test_img[..], ctx.primary_item.as_slice());
    assert_eq!(&test_alpha[..], ctx.alpha_item.as_deref().unwrap());
}