use chad::{RenderContext, wgpu};
use sanscale::{Align, Color, Draw, Rect, Style, TextService, Vec2};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tau_markdown::{Document, Faces, Preview, Theme};
use wgpu::util::DeviceExt;

pub fn color(hex: u32) -> Color {
    let channel = |n: u32| {
        let x = n as f32 / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    Color([
        channel(hex >> 16 & 255),
        channel(hex >> 8 & 255),
        channel(hex & 255),
        1.,
    ])
}
pub fn contains(r: Rect, p: Vec2) -> bool {
    r.width > 0.
        && r.height > 0.
        && p.x >= r.x
        && p.y >= r.y
        && p.x <= r.x + r.width
        && p.y <= r.y + r.height
}
/// Half-open bounds give adjacent sections an unambiguous pointer/copy boundary.
/// Corners are ordered top-left, top-right, bottom-right, bottom-left.
pub fn contains_rounded(rect: Rect, corners: [f32; 4], point: Vec2) -> bool {
    if !contains(rect, point) || point.x >= rect.x + rect.width || point.y >= rect.y + rect.height {
        return false;
    }
    let x = point.x - rect.x - rect.width * 0.5;
    let y = point.y - rect.y - rect.height * 0.5;
    let index = match (x > 0., y > 0.) {
        (false, false) => 0,
        (true, false) => 1,
        (true, true) => 2,
        (false, true) => 3,
    };
    let r = corners[index].clamp(0., rect.width.min(rect.height) * 0.5);
    let qx = x.abs() - rect.width * 0.5 + r;
    let qy = y.abs() - rect.height * 0.5 + r;
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) <= r
}
pub fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(
        x,
        y,
        (a.x + a.width).min(b.x + b.width).max(x) - x,
        (a.y + a.height).min(b.y + b.height).max(y) - y,
    )
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
    local: [f32; 2],
    half_size: [f32; 2],
    corners: [f32; 4],
}
#[derive(Clone, Copy, Default)]
pub struct Interaction {
    pub hover: Option<Vec2>,
    pub pressed: Option<Vec2>,
    pub held: bool,
}
struct Shape {
    rect: Rect,
    clip: Rect,
    color: Color,
    corners: [f32; 4],
}
#[derive(Default)]
pub struct Layer {
    rects: Vec<Shape>,
    pub interaction: Interaction,
    pub draws: Vec<Draw>,
    pub images: Vec<(PathBuf, Rect, Rect)>,
}
impl Layer {
    pub fn new(interaction: Interaction) -> Self {
        Self {
            interaction,
            ..Default::default()
        }
    }
    pub fn rect(&mut self, rect: Rect, color: Color) {
        self.rounded_rect(rect, 0., color);
    }
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.clipped_rounded_rect(rect, radius, color, rect);
    }
    pub fn clipped_rect(&mut self, rect: Rect, color: Color, clip: Rect) {
        self.clipped_rounded_rect(rect, 0., color, clip);
    }
    pub fn clipped_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color, clip: Rect) {
        self.clipped_corners(rect, [radius; 4], color, clip);
    }
    pub fn clipped_corners(&mut self, rect: Rect, corners: [f32; 4], color: Color, clip: Rect) {
        let clip = intersect(rect, clip);
        if clip.width > 0. && clip.height > 0. {
            self.rects.push(Shape {
                rect,
                clip,
                color,
                corners: corners.map(|r| r.clamp(0., rect.width.min(rect.height) * 0.5)),
            });
        }
    }
    pub fn control_color(&self, rect: Rect, base: Color) -> Color {
        self.surface_color(rect, [0.; 4], rect, base)
    }
    /// Tint the entire section after its panels, but below text/images. Using
    /// the same full shape prevents square hover patches at rounded corners.
    pub fn surface_highlight(&mut self, rect: Rect, corners: [f32; 4], clip: Rect, pinned: bool) {
        let strength = if pinned {
            0.035
        } else {
            self.surface_color(rect, corners, clip, Color([0., 0., 0., 1.]))
                .0[0]
        };
        if strength > 0. {
            self.clipped_corners(rect, corners, Color([1., 1., 1., strength]), clip);
        }
    }
    pub fn surface_color(
        &self,
        rect: Rect,
        corners: [f32; 4],
        clip: Rect,
        mut base: Color,
    ) -> Color {
        let inside = |p| contains(clip, p) && contains_rounded(rect, corners, p);
        let input = self.interaction;
        if input.hover.is_some_and(inside) {
            let mix = if input.pressed.is_some_and(inside) {
                0.09
            } else if !input.held {
                0.035
            } else {
                0.
            };
            for c in &mut base.0[..3] {
                *c += (1. - *c) * mix;
            }
        }
        base
    }
}
pub struct MessageView {
    pub source: String,
    pub doc: Document,
    pub view: Preview,
}
impl MessageView {
    pub fn update(&mut self, source: &str) {
        if self.source == source {
            return;
        }
        if let Some(tail) = source.strip_prefix(&self.source) {
            self.doc.append(tail).expect("valid UTF-8 append");
        } else {
            let mut prefix = self
                .source
                .bytes()
                .zip(source.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            while !source.is_char_boundary(prefix) {
                prefix -= 1;
            }
            self.doc
                .edit(prefix..self.source.len(), &source[prefix..])
                .expect("UTF-8 suffix replacement");
        }
        self.source = source.into();
    }
}
struct Image {
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
}
#[derive(Clone, PartialEq, Eq)]
pub struct TextPoint {
    pub key: String,
    pub byte: usize,
}
#[derive(Clone)]
pub struct Selection {
    pub anchor: TextPoint,
    pub focus: TextPoint,
}
pub struct Renderer {
    pub text: TextService,
    pub faces: Faces,
    pub messages: HashMap<String, MessageView>,
    next_namespace: u32,
    pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    image_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    images: HashMap<PathBuf, Image>,
    icons: HashMap<PathBuf, (u64, Image)>,
    scenes: HashMap<String, (tau_markdown::Scene, Rect)>,
    pub selection: Option<Selection>,
    message_order: Vec<String>,
}
impl Renderer {
    pub fn new(ctx: &impl RenderContext) -> Result<Self, String> {
        let mut text = TextService::new();
        let faces = crate::fonts::load(&mut text)?;
        text.set_target(ctx.device(), ctx.format());
        let shader = ctx
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Tau shapes"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline = ctx
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Tau shapes"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format(),
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let image_layout =
            ctx.device()
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Tau image"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let layout = ctx
            .device()
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&image_layout)],
                immediate_size: 0,
            });
        let image_shader = ctx
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Tau image"),
                source: wgpu::ShaderSource::Wgsl(IMAGE_SHADER.into()),
            });
        let image_pipeline = ctx
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Tau image"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &image_shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &image_shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format(),
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let sampler = ctx.device().create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            text,
            faces,
            messages: HashMap::new(),
            next_namespace: 1,
            pipeline,
            image_pipeline,
            image_layout,
            sampler,
            images: HashMap::new(),
            icons: HashMap::new(),
            scenes: HashMap::new(),
            selection: None,
            message_order: vec![],
        })
    }
    pub fn label(
        &mut self,
        layer: &mut Layer,
        value: &str,
        rect: Rect,
        size: f32,
        color: Color,
        bold: bool,
    ) -> f32 {
        self.clipped_label(layer, value, rect, size, color, bold, rect)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn clipped_label(
        &mut self,
        layer: &mut Layer,
        value: &str,
        rect: Rect,
        size: f32,
        color: Color,
        bold: bool,
        clip: Rect,
    ) -> f32 {
        let style = Style {
            chain: self.faces.prose[usize::from(bold)],
            wrap_em: Some(rect.width.max(1.) / size),
            align: Align::Left,
            line_spacing: 1.15,
        };
        if let Some(block) = self.text.shape_transient(value, &style) {
            let height = self.text.measure(block).height_em() * size;
            layer.draws.push(Draw {
                block,
                at: Vec2::new(rect.x, rect.y),
                size,
                color,
                clip: Some(intersect(rect, clip)),
                ..Default::default()
            });
            height
        } else {
            0.
        }
    }
    pub fn message_height(&mut self, key: &str, source: &str, width: f32, size: f32) -> f32 {
        self.message_order.push(key.to_owned());
        let message = self.messages.entry(key.into()).or_insert_with(|| {
            let namespace = self.next_namespace;
            self.next_namespace += 1;
            MessageView {
                source: String::new(),
                doc: Document::new(""),
                view: Preview::new(namespace),
            }
        });
        message.update(source);
        message.view.sync(
            &message.doc,
            &mut self.text,
            self.faces,
            Theme::default(),
            width.max(1.),
            size,
        );
        message.view.height
    }
    pub fn message(
        &mut self,
        layer: &mut Layer,
        key: &str,
        at: Vec2,
        viewport: Rect,
        horizontal: f32,
    ) {
        let selection = self.selection_range(key);
        let Some(message) = self.messages.get_mut(key) else {
            return;
        };
        let mut scene = message.view.scene(
            &mut self.text,
            &message.doc,
            viewport,
            Vec2::new(horizontal, viewport.y - at.y),
        );
        for decoration in scene.under.drain(..).chain(scene.over.drain(..)) {
            layer.clipped_rect(decoration.rect, decoration.color, viewport);
        }
        if let Some(range) = selection {
            for decoration in message
                .view
                .selection(&scene, &self.text, &message.doc, range)
            {
                layer.clipped_rect(decoration.rect, decoration.color, viewport);
            }
        }
        layer.draws.append(&mut scene.draws);
        self.scenes.insert(key.into(), (scene, viewport));
    }
    pub fn clear_scenes(&mut self) {
        self.scenes.clear();
        self.message_order.clear();
    }
    pub fn hit_text(&self, point: Vec2) -> Option<(String, usize)> {
        for (key, (scene, viewport)) in &self.scenes {
            if !contains(*viewport, point) {
                continue;
            }
            let m = &self.messages[key];
            if let Some(byte) = m.view.hit_source(scene, point, &self.text, &m.doc) {
                return Some((key.clone(), byte));
            }
        }
        None
    }
    pub fn hit_link(&self, point: Vec2) -> Option<String> {
        for (key, (scene, viewport)) in &self.scenes {
            if contains(*viewport, point)
                && let Some(link) = self.messages[key].view.hit_link(scene, point, &self.text)
            {
                return Some(link);
            }
        }
        None
    }
    pub fn nearest_text(&self, point: Vec2) -> Option<TextPoint> {
        let mut nearest: Option<(TextPoint, f32, f32)> = None;
        for key in &self.message_order {
            let Some((scene, _)) = self.scenes.get(key) else {
                continue;
            };
            let m = &self.messages[key];
            if let Some((byte, dy, dx)) = m.view.nearest_source(scene, point, &self.text, &m.doc)
                && nearest
                    .as_ref()
                    .is_none_or(|(_, y, x)| dy < *y || dy == *y && dx < *x)
            {
                nearest = Some((
                    TextPoint {
                        key: key.clone(),
                        byte,
                    },
                    dy,
                    dx,
                ));
            }
        }
        nearest.map(|(point, _, _)| point)
    }
    pub fn begin_selection(&mut self, point: TextPoint) {
        self.selection = Some(Selection {
            anchor: point.clone(),
            focus: point,
        });
    }
    pub fn extend_selection(&mut self, point: TextPoint) -> bool {
        if let Some(selection) = &mut self.selection
            && selection.focus != point
        {
            selection.focus = point;
            return true;
        }
        false
    }
    fn selection_range(&self, key: &str) -> Option<std::ops::Range<usize>> {
        let s = self.selection.as_ref()?;
        let a = self.message_order.iter().position(|k| k == &s.anchor.key)?;
        let b = self.message_order.iter().position(|k| k == &s.focus.key)?;
        let i = self.message_order.iter().position(|k| k == key)?;
        let (start, end) = if (a, s.anchor.byte) <= (b, s.focus.byte) {
            (&s.anchor, &s.focus)
        } else {
            (&s.focus, &s.anchor)
        };
        if i < a.min(b) || i > a.max(b) {
            return None;
        }
        let len = self.messages.get(key)?.source.len();
        let lo = if start.key == key {
            start.byte.min(len)
        } else {
            0
        };
        let hi = if end.key == key {
            end.byte.min(len)
        } else {
            len
        };
        (lo < hi).then_some(lo..hi)
    }
    pub fn selected_text(&self) -> Option<String> {
        self.selection.as_ref()?;
        let parts = self
            .message_order
            .iter()
            .filter_map(|key| {
                let range = self.selection_range(key)?;
                let m = &self.messages[key];
                Some(m.view.copy_selection(&m.doc, range))
            })
            .collect::<Vec<_>>();
        Some(parts.join("\n\n"))
    }
    pub fn retain_messages(&mut self, keys: &std::collections::HashSet<String>) {
        if self
            .selection
            .as_ref()
            .is_some_and(|s| !keys.contains(&s.anchor.key) || !keys.contains(&s.focus.key))
        {
            self.selection = None;
        }
        self.messages.retain(|key, m| {
            if keys.contains(key) {
                true
            } else {
                m.view.release(&mut self.text);
                false
            }
        });
    }
    pub fn image_size(
        &mut self,
        ctx: &impl RenderContext,
        path: &Path,
    ) -> Result<(u32, u32), String> {
        if let Some(image) = self.images.get(path) {
            return Ok((image.width, image.height));
        }
        let reader = image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut reader = reader;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(64 * 1024 * 1024);
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|e| e.to_string())?;
        let decoded = if decoded.width() > 2048 || decoded.height() > 2048 {
            decoded.thumbnail(2048, 2048)
        } else {
            decoded
        };
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        let image = self.upload_image(ctx, &rgba, width, height);
        // Bound decoded GPU images, not the verified originals on disk.
        if self.images.len() >= 16 {
            self.images.clear();
        }
        self.images.insert(path.to_owned(), image);
        Ok((width, height))
    }
    fn upload_image(
        &self,
        ctx: &impl RenderContext,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Image {
        let texture = ctx.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("Tau preview"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue().write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
        let bind = ctx.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Image {
            bind,
            width,
            height,
        }
    }
    pub fn icon(
        &mut self,
        ctx: &impl RenderContext,
        layer: &mut Layer,
        icon: crate::icons::Icon,
        rect: Rect,
        color: u32,
    ) {
        let size = rect.width.ceil().max(1.) as u32;
        let key = PathBuf::from(format!("tau-icon/{}/{}", icon.name(), size));
        let stamp = icon.stamp(color);
        if self.icons.get(&key).is_none_or(|(old, _)| *old != stamp) {
            let rgba = icon.pixels(size, color);
            let image = self.upload_image(ctx, &rgba, size, size);
            self.icons.insert(key.clone(), (stamp, image));
        }
        layer.images.push((key, rect, rect));
    }
    pub fn draw(&mut self, ctx: &impl RenderContext, target: &wgpu::TextureView, layers: &[Layer]) {
        let (width, height) = ctx.size();
        self.text
            .set_transform(ctx.queue(), TextService::pixel_ortho(width, height));
        let ndc = |x: f32, y: f32| [x / width as f32 * 2. - 1., 1. - y / height as f32 * 2.];
        let mut batches = vec![];
        let mut shapes = vec![];
        let mut image_buffers = vec![];
        for layer in layers {
            batches.push(self.text.prepare(ctx.device(), ctx.queue(), &layer.draws));
            let mut vertices = vec![];
            for shape in &layer.rects {
                let r = shape.clip;
                let full = shape.rect;
                for [x, y] in [
                    [r.x, r.y],
                    [r.x + r.width, r.y],
                    [r.x, r.y + r.height],
                    [r.x, r.y + r.height],
                    [r.x + r.width, r.y],
                    [r.x + r.width, r.y + r.height],
                ] {
                    vertices.push(Vertex {
                        position: ndc(x, y),
                        color: shape.color.0,
                        local: [
                            x - full.x - full.width * 0.5,
                            y - full.y - full.height * 0.5,
                        ],
                        half_size: [full.width * 0.5, full.height * 0.5],
                        corners: shape.corners,
                    });
                }
            }
            shapes.push((
                vertices.len() as u32,
                ctx.device()
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            ));
            let buffers = layer
                .images
                .iter()
                .map(|(_, r, _)| {
                    let verts = [
                        [r.x, r.y, 0., 0.],
                        [r.x + r.width, r.y, 1., 0.],
                        [r.x, r.y + r.height, 0., 1.],
                        [r.x, r.y + r.height, 0., 1.],
                        [r.x + r.width, r.y, 1., 0.],
                        [r.x + r.width, r.y + r.height, 1., 1.],
                    ]
                    .map(|[x, y, u, v]| {
                        let p = ndc(x, y);
                        [p[0], p[1], u, v]
                    });
                    ctx.device()
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: None,
                            contents: bytemuck::cast_slice(&verts),
                            usage: wgpu::BufferUsages::VERTEX,
                        })
                })
                .collect::<Vec<_>>();
            image_buffers.push(buffers);
        }
        let mut encoder = ctx.device().create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Tau"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0027,
                            g: 0.004,
                            b: 0.006,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for (i, layer) in layers.iter().enumerate() {
                pass.set_scissor_rect(0, 0, width, height);
                if shapes[i].0 > 0 {
                    pass.set_pipeline(&self.pipeline);
                    pass.set_vertex_buffer(0, shapes[i].1.slice(..));
                    pass.draw(0..shapes[i].0, 0..1);
                }
                for (j, (path, _, clip)) in layer.images.iter().enumerate() {
                    if let Some(image) = self
                        .images
                        .get(path)
                        .or_else(|| self.icons.get(path).map(|(_, image)| image))
                        && let Some([x, y, w, h]) = scissor(*clip, width, height)
                    {
                        pass.set_scissor_rect(x, y, w, h);
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &image.bind, &[]);
                        pass.set_vertex_buffer(0, image_buffers[i][j].slice(..));
                        pass.draw(0..6, 0..1);
                    }
                }
                for (index, segment) in batches[i].segments().iter().enumerate() {
                    let clip =
                        segment
                            .clip
                            .unwrap_or(Rect::new(0., 0., width as f32, height as f32));
                    if let Some([x, y, w, h]) = scissor(clip, width, height) {
                        pass.set_scissor_rect(x, y, w, h);
                        self.text.draw_segment(&mut pass, &batches[i], index);
                    }
                }
            }
        }
        ctx.queue().submit([encoder.finish()]);
    }
}
fn scissor(r: Rect, width: u32, height: u32) -> Option<[u32; 4]> {
    let x = r.x.floor().max(0.) as u32;
    let y = r.y.floor().max(0.) as u32;
    let right = (r.x + r.width).ceil().max(0.) as u32;
    let bottom = (r.y + r.height).ceil().max(0.) as u32;
    let w = right.min(width).saturating_sub(x);
    let h = bottom.min(height).saturating_sub(y);
    (w > 0 && h > 0).then_some([x, y, w, h])
}
const SHADER: &str = r#"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) corners: vec4<f32>,
}
@vertex fn vs(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>,
              @location(2) local: vec2<f32>, @location(3) half_size: vec2<f32>,
              @location(4) corners: vec4<f32>) -> Out {
    return Out(vec4<f32>(pos,0.,1.), color, local, half_size, corners);
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let top = select(in.corners.x, in.corners.y, in.local.x > 0.);
    let bottom = select(in.corners.w, in.corners.z, in.local.x > 0.);
    let radius = select(top, bottom, in.local.y > 0.);
    let q = abs(in.local) - in.half_size + vec2<f32>(radius);
    let distance = length(max(q, vec2<f32>(0.))) + min(max(q.x, q.y), 0.) - radius;
    let coverage = clamp(0.5 - distance / max(fwidth(distance), 1.), 0., 1.);
    return vec4<f32>(in.color.rgb, in.color.a * select(1., coverage, radius > 0.));
}
"#;
const IMAGE_SHADER: &str = r#"
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@vertex fn vs(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> Out { return Out(vec4<f32>(pos,0.,1.),uv); }
@fragment fn fs(in: Out) -> @location(0) vec4<f32> { return textureSample(image,image_sampler,in.uv); }
"#;
