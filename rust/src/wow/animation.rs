use deku::prelude::*;
use wasm_bindgen::prelude::*;
use js_sys::Float32Array;
use crate::wow::m2::*;
use crate::wow::common::*;

#[derive(Debug, Clone)]
struct LcgRng {
    state: u32,
}

impl LcgRng {
    pub fn new(seed: u32) -> Self {
        LcgRng { state: seed }
    }

    pub fn next_u16(&mut self) -> u16 {
        self.state = self.state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.state %= 1 << 31;
        self.state as u16
    }

    pub fn next_f32(&mut self) -> f32 {
        self.next_u16() as f32 / std::u16::MAX as f32
    }
}

#[derive(DekuRead, Debug, Clone)]
pub struct M2CompBoneUnallocated {
    pub key_bone_id: i32,
    pub flags: u32,
    pub parent_bone: i16,
    pub submesh_id: u16,
    pub bone_name_crc: u32,
    pub translation: M2TrackUnallocated<Vec3>,
    pub rotation: M2TrackUnallocated<Quat16>,
    pub scaling: M2TrackUnallocated<Vec3>,
    pub pivot: Vec3,
}

#[derive(Debug, Clone)]
pub struct M2CompBone {
    pub key_bone_id: i32,
    pub flags: u32,
    pub parent_bone: i16,
    pub submesh_id: u16,
    pub translation: M2Track<Vec3>,
    pub rotation: M2Track<Quat>,
    pub scaling: M2Track<Vec3>,
    pub pivot: Vec3,
}

#[derive(DekuRead, Debug, Clone)]
pub struct M2Sequence {
    pub id: u16, // lookup table id?
    pub sub_id: u16, // which number in a row of animations this one is
    pub duration: u32, // in milliseconds
    pub movespeed: f32, // speed the character moves with in the animation
    pub flags: u32,
    #[deku(pad_bytes_after = "2")]
    pub frequency: u16, // how often this should be played (for all animations of the same type, this adds up to 0x7fff)
    pub replay_min: u32,
    pub replay_max: u32,
    pub blend_time: u32,
    pub bounds_aabb: AABBox,
    pub bounds_radius: f32,
    pub variation_next: i16, // id of the next animation of this animation id, -1 if none
    pub alias_next: u16, // id in the list of animations, used to find actual animation if this sequence is an alias (flags & 0x40)
}

#[derive(DekuRead, Debug, Clone)]
pub struct M2TrackUnallocated<T> {
    pub interpolation_type: u16,
    pub global_sequence: i16,
    pub timestamps: WowArray<WowArray<u32>>,
    pub values: WowArray<WowArray<T>>,
}

impl<T> M2TrackUnallocated<T> {
    pub fn to_allocated(&self, data: &[u8]) -> Result<M2Track<T>, DekuError>
        where for<'a> T: DekuRead<'a> {
        let mut timestamps = Vec::new();
        for arr in self.timestamps.to_vec(data)? {
            timestamps.push(arr.to_vec(data)?);
        }

        let mut values = Vec::new();
        for arr in self.values.to_vec(data)? {
            values.push(arr.to_vec(data)?);
        }

        Ok(M2Track {
            interpolation_type: self.interpolation_type,
            global_sequence: self.global_sequence,
            timestamps,
            values,
        })
    }
}

#[derive(DekuRead, Debug, Clone)]
pub struct M2TextureTransformUnallocated {
    pub translation: M2TrackUnallocated<Vec3>,
    pub rotation: M2TrackUnallocated<Quat>,
    pub scaling: M2TrackUnallocated<Vec3>,
}

#[derive(DekuRead, Debug, Clone)]
pub struct M2ColorUnallocated {
    pub color: M2TrackUnallocated<Vec3>, // rgb
    pub alpha: M2TrackUnallocated<u16>, // 0 = transparent, 0x7FFF = opaque
}

#[derive(Debug, Clone)]
pub struct M2Track<T> {
    pub interpolation_type: u16,
    pub global_sequence: i16,
    pub timestamps: Vec<Vec<u32>>,
    pub values: Vec<Vec<T>>,
}

#[derive(Debug, Clone)]
pub struct M2TextureTransform {
    pub translation: M2Track<Vec3>,
    pub rotation: M2Track<Quat>,
    pub scaling: M2Track<Vec3>,
}

#[derive(Debug, Clone)]
pub struct M2TextureWeight {
    pub weights: M2Track<u16>,
}

#[derive(Debug, Clone)]
pub struct M2Color {
    pub color: M2Track<Vec3>,
    pub alpha: M2Track<u16>,
}

#[derive(Debug, Clone)]
pub struct AnimationState {
    pub animation_index: Option<usize>,
    pub repeat_times: i32,
    pub animation_time: f64,
    pub animation_record: Option<M2Sequence>,
    pub main_variation_index: usize,
    pub main_variation_record: Option<M2Sequence>,
}

impl AnimationState {
    fn new(first_sequence: Option<M2Sequence>) -> Self {
        match first_sequence {
            Some(seq) => AnimationState {
                animation_index: Some(0),
                repeat_times: 0,
                animation_time: 0.0,
                animation_record: Some(seq.clone()),
                main_variation_index: 0,
                main_variation_record: Some(seq),
            },
            None => AnimationState {
                animation_index: None,
                repeat_times: 0,
                animation_time: 0.0,
                animation_record: None,
                main_variation_index: 0,
                main_variation_record: None,
            }
        }
    }

    fn calculate_animation_repeats(&mut self, rng: &mut LcgRng) {
        if let Some(record) = &self.animation_record {
            let times = (record.replay_max - record.replay_min) as f32;
            self.repeat_times = record.replay_min as i32 + (times * rng.next_f32()) as i32;
        }
    }
}

#[wasm_bindgen(js_name = "WowM2AnimationManager")]
#[derive(Debug, Clone)]
pub struct AnimationManager {
    global_sequence_durations: Vec<u32>,
    global_sequence_times: Vec<f64>,
    sequences: Vec<M2Sequence>,
    texture_weights: Vec<M2TextureWeight>,
    texture_transforms: Vec<M2TextureTransform>,
    current_animation: AnimationState,
    next_animation: AnimationState,
    rng: LcgRng,
    blend_factor: f32,
    colors: Vec<M2Color>,
    bones: Vec<M2CompBone>,

    calculated_transparencies: Vec<f32>,
    calculated_texture_translations: Vec<Vec3>,
    calculated_texture_rotations: Vec<Quat>,
    calculated_texture_scalings: Vec<Vec3>,
    calculated_colors: Vec<Vec4>,
    calculated_bone_translations: Vec<Vec3>,
    calculated_bone_rotations: Vec<Quat>,
    calculated_bone_scalings: Vec<Vec3>,
}

#[wasm_bindgen(js_class = "WowM2AnimationManager")]
impl AnimationManager {
    pub fn update_animations(
        &mut self, delta_time: f64,
        transparencies: &Float32Array,
        texture_translations: &Float32Array,
        texture_rotations: &Float32Array,
        texture_scalings: &Float32Array,
        bone_translations: &Float32Array,
        bone_rotations: &Float32Array,
        bone_scalings: &Float32Array,
        colors: &Float32Array
    ) {
        self.update(delta_time);
        transparencies.copy_from(&self.calculated_transparencies);
        for i in 0..self.texture_transforms.len() {
            let translation_index = i as u32 * 3;
            let translation = &self.calculated_texture_translations[i];
            texture_translations.set_index(translation_index, translation.x);
            texture_translations.set_index(translation_index + 1, translation.y);
            texture_translations.set_index(translation_index + 2, translation.z);

            let rotation_index = i as u32 * 4;
            let rotation = &self.calculated_texture_rotations[i];
            texture_rotations.set_index(rotation_index, rotation.x);
            texture_rotations.set_index(rotation_index + 1, rotation.y);
            texture_rotations.set_index(rotation_index + 2, rotation.z);
            texture_rotations.set_index(rotation_index + 3, rotation.w);

            let scaling_index = i as u32 * 3;
            let scaling = &self.calculated_texture_scalings[i];
            texture_scalings.set_index(scaling_index, scaling.x);
            texture_scalings.set_index(scaling_index + 1, scaling.y);
            texture_scalings.set_index(scaling_index + 2, scaling.z);
        }
        for i in 0..self.bones.len() {
            let translation_index = i as u32 * 3;
            let translation = &self.calculated_bone_translations[i];
            bone_translations.set_index(translation_index, translation.x);
            bone_translations.set_index(translation_index + 1, translation.y);
            bone_translations.set_index(translation_index + 2, translation.z);

            let rotation_index = i as u32 * 4;
            let rotation = &self.calculated_bone_rotations[i];
            bone_rotations.set_index(rotation_index, rotation.x);
            bone_rotations.set_index(rotation_index + 1, rotation.y);
            bone_rotations.set_index(rotation_index + 2, rotation.z);
            bone_rotations.set_index(rotation_index + 3, rotation.w);

            let scaling_index = i as u32 * 3;
            let scaling = &self.calculated_bone_scalings[i];
            bone_scalings.set_index(scaling_index, scaling.x);
            bone_scalings.set_index(scaling_index + 1, scaling.y);
            bone_scalings.set_index(scaling_index + 2, scaling.z);
        }
        for i in 0..self.colors.len() {
            let color_index = i as u32 * 4;
            let color = &self.calculated_colors[i];
            colors.set_index(color_index, color.x);
            colors.set_index(color_index + 1, color.y);
            colors.set_index(color_index + 2, color.z);
            colors.set_index(color_index + 3, color.w);
        }
    }

    pub fn get_num_colors(&self) -> usize {
        self.colors.len()
    }

    pub fn get_num_bones(&self) -> usize {
        self.bones.len()
    }

    pub fn get_bone_pivots(&self) -> Vec<Vec3> {
        self.bones.iter().map(|bone| bone.pivot).collect()
    }

    pub fn get_bone_parents(&self) -> Vec<i16> {
        self.bones.iter().map(|bone| bone.parent_bone).collect()
    }

    pub fn get_bone_flags(&self) -> Vec<M2BoneFlags> {
        self.bones.iter().map(|bone| M2BoneFlags::new(bone.flags)).collect()
    }

    pub fn get_num_transformations(&self) -> usize {
        self.texture_transforms.len()
    }

    pub fn get_num_texture_weights(&self) -> usize {
        self.texture_weights.len()
    }
}

// rust-only interface
impl AnimationManager {
    pub fn new(
        global_sequence_durations: Vec<u32>,
        sequences: Vec<M2Sequence>,
        texture_weights: Vec<M2TextureWeight>,
        texture_transforms: Vec<M2TextureTransform>,
        colors: Vec<M2Color>,
        bones: Vec<M2CompBone>,
    ) -> Self {
        let global_sequence_times = vec![0.0; global_sequence_durations.len()];
        let mut current_animation = AnimationState::new(Some(sequences[0].clone()));
        let mut rng = LcgRng::new(1312);
        current_animation.calculate_animation_repeats(&mut rng);
        let next_animation = AnimationState::new(None);

        let calculated_transparencies: Vec<f32> = Vec::with_capacity(texture_weights.len());
        let calculated_texture_translations: Vec<Vec3> = Vec::with_capacity(texture_transforms.len());
        let calculated_texture_rotations: Vec<Quat> = Vec::with_capacity(texture_transforms.len());
        let calculated_texture_scalings: Vec<Vec3> = Vec::with_capacity(texture_transforms.len());
        let calculated_colors: Vec<Vec4> = Vec::with_capacity(colors.len());

        let calculated_bone_translations: Vec<Vec3> = Vec::with_capacity(bones.len());
        let calculated_bone_rotations: Vec<Quat> = Vec::with_capacity(bones.len());
        let calculated_bone_scalings: Vec<Vec3> = Vec::with_capacity(bones.len());

        AnimationManager {
            global_sequence_durations,
            current_animation,
            next_animation,
            blend_factor: 0.0,
            sequences,
            texture_transforms,
            texture_weights,
            colors,
            bones,
            global_sequence_times,
            rng,
            calculated_transparencies,
            calculated_texture_translations,
            calculated_texture_rotations,
            calculated_texture_scalings,
            calculated_colors,
            calculated_bone_translations,
            calculated_bone_rotations,
            calculated_bone_scalings,
        }
    }

    fn get_current_value<T>(&self, mut curr_time: f64, mut animation_index: usize, animation: &M2Track<T>, default: T) -> T
        where T: Clone + Lerp
        {
        let mut max_time = self.sequences[animation_index].duration;

        if animation.global_sequence >= 0 {
            curr_time = self.global_sequence_times[animation.global_sequence as usize];
            max_time = self.global_sequence_durations[animation.global_sequence as usize];
        }

        if animation.timestamps.len() <= animation_index {
            animation_index = 0;
        }

        if animation.timestamps.is_empty() {
            return default;
        }

        if animation_index <= animation.timestamps.len() && animation.timestamps[animation_index].is_empty() {
            return default;
        }

        let times = &animation.timestamps[animation_index];
        let values = &animation.values[animation_index];

        // find the highest timestamp still less than curr_time
        let time_index: i32;
        if max_time != 0 {
            if times.len() > 1 {
                let last_index = times.len() - 1;
                if curr_time > times[last_index] as f64 {
                    time_index = last_index as i32;
                } else {
                    let next_timestamp_idx = times.iter().position(|time| {
                        *time as f64 >= curr_time
                    }).unwrap() as i32;
                    if next_timestamp_idx != 0 {
                        time_index = next_timestamp_idx - 1;
                    } else {
                        time_index = next_timestamp_idx;
                    }
                }
            } else if times.len() == 1 {
                time_index = 0;
            } else {
                time_index = -1;
            }
        } else {
            time_index = 0;
        }

        if time_index as usize == times.len() - 1 {
            values[time_index as usize].clone()
        } else if time_index >= 0 {
            let value1 = &values[time_index as usize];
            let value2 = &values[time_index as usize + 1];
            let time1 = times[time_index as usize];
            let time2 = times[time_index as usize + 1];

            if animation.interpolation_type == 0 {
                return value1.clone();
            } else if animation.interpolation_type == 1 {
                let t = (curr_time - time1 as f64) / (time2 as f64 - time1 as f64);
                return value1.clone().lerp(value2.clone(), t as f32);
            } else {
                unreachable!("unknown interpolation type!")
            }
        } else {
            return values[0].clone();
        }
    }

    fn get_current_value_with_blend<T>(&self, animation: &M2Track<T>, default: T) -> T
        where T: Clone + Lerp {
        let result = self.get_current_value(
            self.current_animation.animation_time,
            self.current_animation.animation_index.unwrap(),
            animation,
            default.clone()
        );
        
        if self.blend_factor < 0.999 {
            if let Some(next_index) = self.next_animation.animation_index {
                let next_result = self.get_current_value(
                    self.next_animation.animation_time,
                    next_index,
                    animation,
                    default.clone()
                );

                return result.lerp(next_result, self.blend_factor);
            }
        }

        result
    }
}

impl AnimationManager {
    pub fn update(&mut self, delta_time: f64) {
        self.current_animation.animation_time += delta_time;

        for i in 0..self.global_sequence_times.len() {
            self.global_sequence_times[i] += delta_time;
            if self.global_sequence_durations[i] > 0 {
                self.global_sequence_times[i] %= self.global_sequence_durations[i] as f64;
            }
        }

        let current_record = self.current_animation.animation_record.as_ref().unwrap();

        // If we don't have a next animation yet, and this animation isn't set
        // to repeat again, choose the next one
        let mut sub_anim_record: Option<&M2Sequence> = None;
        if self.next_animation.animation_index.is_none()
            && self.current_animation.main_variation_record.as_ref().unwrap().variation_next > -1
            && self.current_animation.repeat_times <= 0 {

            let probability = (self.rng.next_f32() * 0x7fff as f32) as u16;
            let mut calc_prob = 0;

            let mut next_index = self.current_animation.main_variation_index;
            let mut next_record = &self.sequences[next_index];
            calc_prob += next_record.frequency;
            while calc_prob < probability && next_record.variation_next > -1 {
                next_index = next_record.variation_next as usize;
                next_record = &self.sequences[next_index];

                if self.current_animation.animation_index != Some(next_index) {
                    calc_prob += next_record.frequency;
                }
            }
            sub_anim_record = Some(next_record);

            self.next_animation.animation_index = Some(next_index);
            self.next_animation.animation_record = Some(next_record.clone());
            self.next_animation.animation_time = 0.0;
            self.next_animation.main_variation_index = self.current_animation.main_variation_index;
            self.next_animation.main_variation_record = self.current_animation.main_variation_record.clone();
            self.next_animation.calculate_animation_repeats(&mut self.rng);
        } else if self.current_animation.repeat_times > 0 {
            self.next_animation = self.current_animation.clone();
            self.next_animation.repeat_times -= 1;
        }

        let current_animation_time_left = current_record.duration as f64 - self.current_animation.animation_time;
        let mut sub_anim_blend_time = 0.0;

        // if we have a next animation stored, get its blend time
        if let Some(next_index) = self.next_animation.animation_index {
            sub_anim_record = Some(&self.sequences[next_index]);
            sub_anim_blend_time = self.sequences[next_index].blend_time as f64;
        }

        // if it's time to start blending into the next animation, setup an appropriate blend factor
        if sub_anim_blend_time > 0.0 && current_animation_time_left < sub_anim_blend_time {
            self.next_animation.animation_time = (sub_anim_blend_time - current_animation_time_left) % sub_anim_record.unwrap().duration as f64;
            self.blend_factor = (current_animation_time_left / sub_anim_blend_time) as f32;
        } else {
            self.blend_factor = 1.0;
        }

        // if the current animation is done and we have a next animation, swap
        // them. otherwise, loop the current one
        if self.current_animation.animation_time >= current_record.duration as f64 {
            self.current_animation.repeat_times -= 1;

            if let Some(index) = self.next_animation.animation_index {
                let mut next_index = index;
                // if the next animation is an alias, look it up
                while ((self.sequences[next_index].flags & 0x20) == 0) && ((self.sequences[next_index].flags & 0x40) > 0) {
                    next_index = self.sequences[next_index].alias_next as usize;
                    if next_index >= self.sequences.len() {
                        break;
                    }
                }
                self.next_animation.animation_index = Some(next_index);
                self.next_animation.animation_record = Some(self.sequences[next_index].clone());

                self.current_animation = self.next_animation.clone();

                self.next_animation.animation_index = None;
                self.next_animation.animation_record = None;
                self.blend_factor = 1.0;
            } else if current_record.duration > 0 {
                self.current_animation.animation_time %= current_record.duration as f64;
            }
        }

        let default_color = Vec3::new(1.0);
        let default_alpha = 0x7fff;
        self.calculated_colors.clear();
        for color in &self.colors {
            let mut rgba = Vec4::new(0.0);
            let rgb = self.get_current_value_with_blend(&color.color, default_color);
            rgba.x = rgb.x;
            rgba.y = rgb.y;
            rgba.z = rgb.z;
            rgba.w = self.get_current_value_with_blend(&color.alpha, default_alpha) as f32 / 0x7fff as f32;
            self.calculated_colors.push(rgba);
        }

        self.calculated_transparencies.clear();
        for weight in &self.texture_weights {
            self.calculated_transparencies.push(self.get_current_value_with_blend(&weight.weights, default_alpha) as f32 / 0x7fff as f32);
        }

        self.calculated_texture_translations.clear();
        let default_translation = Vec3::new(0.0);
        self.calculated_texture_rotations.clear();
        let default_rotation = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
        self.calculated_texture_scalings.clear();
        let default_scaling = Vec3::new(1.0);
        for transform in &self.texture_transforms {
            self.calculated_texture_translations.push(self.get_current_value_with_blend(&transform.translation, default_translation));
            self.calculated_texture_rotations.push(self.get_current_value_with_blend(&transform.rotation, default_rotation));
            self.calculated_texture_scalings.push(self.get_current_value_with_blend(&transform.scaling, default_scaling));
        }

        self.calculated_bone_translations.clear();
        let default_translation = Vec3::new(0.0);
        self.calculated_bone_rotations.clear();
        let default_rotation = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
        self.calculated_bone_scalings.clear();
        let default_scaling = Vec3::new(1.0);
        for bone in &self.bones {
            self.calculated_bone_translations.push(self.get_current_value_with_blend(&bone.translation, default_translation));
            self.calculated_bone_rotations.push(self.get_current_value_with_blend(&bone.rotation, default_rotation));
            self.calculated_bone_scalings.push(self.get_current_value_with_blend(&bone.scaling, default_scaling));
        }
    }
}
