//! 岛仔的原生状态、动作和连续矢量表情。
use crate::anim::{approach, lerp, smoothstep};
use crate::paint::{Card, H};
use crate::theme::{self, W};
use crate::pet_expressions::Expression;
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PetState {
    Awake,
    Curious,
    Waking,
    Happy,
    Satisfied,
    Music,
    Worried,
    Focused,
    Dozing,
    Sleeping,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_gestures_change_rendered_eyes_between_refreshes() {
        use eframe::egui::{self, Pos2, Rect, Vec2};
        let ctx = egui::Context::default();
        crate::theme::load_fonts(&ctx);
        let mut raster = crate::preview_raster::Raster::new(48, 32);
        for (state, start) in [(PetState::Awake, 3.1), (PetState::Waking, 0.65),
            (PetState::Focused, 4.08), (PetState::Dozing, 1.8)] {
            let mut pet = Pet { state, age: start, ..Pet::default() };
            pet.follow_pose(0.0, 2.0, [0.0; 2], true);
            let mut previous = Vec::new();
            let mut changed = 0;
            for _ in 0..24 {
                pet.age += 1.0 / 60.0;
                pet.follow_pose(0.0, 1.0 / 60.0, [0.0; 2], true);
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(48.0, 32.0))), ..Default::default() };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        pet.draw(&Card::new(ui, Pos2::ZERO, ui.max_rect()), 24.0, 0.0, None);
                    });
                });
                raster.update_textures(&output.textures_delta);
                raster.buf.fill([0.0; 4]);
                raster.draw(&ctx.tessellate(output.shapes, 1.0), 1.0);
                let eyes: Vec<_> = raster.buf.chunks_exact(48).skip(6).take(18)
                    .flat_map(|row| row[11..36].iter().map(|p| (p[0] * 255.0).round() as u8))
                    .collect();
                if !previous.is_empty() && eyes != previous { changed += 1; }
                previous = eyes;
                raster.free_textures(&output.textures_delta);
            }
            assert!(changed >= 21, "{state:?}: only {changed} of 23 frame pairs changed");
        }
    }

    #[test]
    fn native_gestures_respect_reduced_motion_visibility_and_gaze() {
        for state in [PetState::Awake, PetState::Waking, PetState::Focused, PetState::Dozing] {
            let mut pet = Pet { state, ..Pet::default() };
            let pose = |p: &Pet| [p.y, p.eyes[0].height, p.eyes[1].height, p.eyes[0].spread, p.eyes[0].y, p.eyes[1].y];
            pet.follow_pose(0.0, 5.0, [0.0; 2], false);
            let still = pose(&pet);
            for n in 0..120 {
                pet.age = n as f32 * 0.15;
                pet.follow_pose(pet.age as f64, 1.0, [0.0; 2], false);
                assert!(pose(&pet).iter().zip(still).all(|(a, b)| (a - b).abs() < 0.0001),
                    "{state:?} should be still with motion off");
            }
            let age = pet.age;
            let before_hide = pose(&pet);
            pet.step(state, 100.0, 60.0, [2.0, 1.0], false);
            assert_eq!((pet.age, pose(&pet), pet.gaze), (age, before_hide, [0.0; 2]));
        }
        let mut pet = Pet::default();
        for (age, gaze) in [(3.8, [-2.0, 1.0]), (15.8, [2.0, -1.0])] {
            pet.age = age;
            pet.follow_pose(0.0, 5.0, gaze, true);
            assert_eq!(pet.gaze, gaze);
            assert!(pet.eyes[0].y * pet.eyes[1].y < -1.0);
        }
    }

    #[test]
    fn open_eyes_do_not_narrow_between_the_caps() {
        use eframe::egui::{self, Pos2, Rect, Vec2};
        for ppp in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for offset in [0.0, 0.25, 0.5, 0.75] {
                let ctx = egui::Context::default();
                ctx.set_zoom_factor(ppp);
                let mut pet = Pet::default();
                pet.gaze = [offset, offset];
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(48.0))), ..Default::default() };
                let out = ctx.run(raw, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        pet.draw(&Card::new(ui, Pos2::ZERO, ui.max_rect()), 24.0, 0.0, None);
                    });
                });
                let side = (48.0 * ppp) as usize;
                let mut raster = crate::preview_raster::Raster::new(side, side);
                raster.update_textures(&out.textures_delta);
                raster.draw(&ctx.tessellate(out.shapes, ppp), ppp);
                for cols in [0..side / 2, side / 2..side] {
                    let width = |y: usize| cols.clone().map(|x| raster.buf[y * side + x][0]).sum::<f32>();
                    let middle = ((14.5 + offset) * ppp).floor() as usize;
                    let widest = (0..side).map(width).fold(0.0, f32::max);
                    assert!(widest > 2.0 && widest - width(middle) < 0.09,
                        "scale={ppp} offset={offset} middle={} widest={widest}", width(middle));
                }
            }
        }
    }
    #[test]
    fn listening_uses_all_twelve_new_expressions_between_singing_poses(){
        let mut pet=Pet::default();let mut seen=std::collections::HashSet::new();
        for frame in 0..15000 {
            pet.step(PetState::Music,frame as f64/60.0,1.0/60.0,[0.0;2],true);
            if let Some(expression)=pet.expressions.current(){seen.insert(expression as usize);}
        }
        assert_eq!(seen.len(),12,"连续听歌应能轮换到十二组新表情");
    }
    #[test]
    fn listening_resumes_the_same_place_after_a_notice_or_pause(){
        let mut pet=Pet::default();
        for frame in 0..1080 {pet.step(PetState::Music,frame as f64/60.0,1.0/60.0,[0.0;2],true);}
        let expression=pet.expressions.current().expect("演唱一段后应穿插新表情");
        pet.step(PetState::Music,30.0,20.0,[0.0;2],false);
        assert_eq!(pet.expressions.current(),Some(expression),"展开面板应暂停轮换");
        for state in [PetState::Awake,PetState::Focused,PetState::Worried] {
            pet.step(state,31.0,1.0,[0.0;2],true);assert!(pet.expressions.current().is_none());
            pet.step(PetState::Music,42.0,0.0,[0.0;2],true);
            assert_eq!(pet.expressions.current(),Some(expression),"恢复听歌应从暂停处继续");
        }
        pet.react(PetState::Satisfied);pet.step(PetState::Satisfied,43.0,0.1,[0.0;2],true);
        assert_eq!(pet.expressions.current(),Some(crate::pet_expressions::Expression::Heart),"喝水反馈应保留优先级");
    }
    #[test]
    fn singing_keeps_each_expression_for_fifteen_seconds_after_the_transition() {
        let mut pet=Pet::default();
        pet.step(PetState::Music,0.0,0.0,[0.0;2],true);
        let pose=pet.music_pose();
        for frame in 1..960 {
            pet.step(PetState::Music,frame as f64/60.0,1.0/60.0,[0.0;2],true);
            assert_eq!(pet.music_pose(),pose,"一秒过渡和十五秒停留期间不能频繁换表情");
        }
        pet.step(PetState::Music,16.1,0.2,[0.0;2],true);
        assert_eq!(pet.expressions.current(),Some(Expression::Wink),"完整演唱一段后再穿插新表情");
        pet.step(PetState::Music,19.4,3.3,[0.0;2],true);
        assert_ne!(pet.music_pose(),pose);
    }
    #[test]
    fn singing_rotates_expressions_and_starts_with_the_microphone() {
        let mut pet=Pet::default();let mut poses=Vec::new();let mut eyes=Vec::new();
        let mut start=0.0;
        for expression in MUSIC_EXPRESSIONS.into_iter().take(7) {
            pet.music_age=start;
            for frame in 0..60 {pet.step(PetState::Music,frame as f64/60.0,1.0/60.0,[0.0;2],true);}
            poses.push(pet.music_pose());eyes.push(pet.eyes.map(|eye|(eye.height,eye.curve)));
            start+=MUSIC_PERIOD+expression.duration();
        }
        assert_eq!(poses,vec![MusicPose::Microphone,MusicPose::Hum,MusicPose::SideSing,MusicPose::Squint,MusicPose::HighNote,MusicPose::Smile,MusicPose::Microphone]);
        assert_ne!(eyes[0],eyes[2]);assert_ne!(eyes[2],eyes[3]);assert_ne!(eyes[3],eyes[4]);
        let age=pet.age;
        let music_age=pet.music_age;
        pet.step(PetState::Music,100.0,20.0,[0.0;2],false);
        assert_eq!(pet.age,age,"展开面板时不应跳过演唱表情");
        assert_eq!(pet.music_age,music_age);
        pet.step(PetState::Awake,101.0,0.1,[0.0;2],true);
        pet.step(PetState::Music,102.0,0.1,[0.0;2],true);
        assert!((pet.music_age-music_age-0.1).abs()<0.001,"恢复听歌后继续轮换");
    }
    #[test]
    fn microphone_moves_toward_the_mouth_and_lowers_for_breaths() {
        let mut heights=Vec::new();let mut angles=Vec::new();
        for frame in 0..600 {
            let (head,angle)=SingingMotion::at(frame as f32/60.0, true).microphone();
            let tip=[head[0]+7.0*angle.cos(),head[1]+7.0*angle.sin()];
            let distance=|p:[f32;2]|((p[0]+0.8).powi(2)+(p[1]-20.7).powi(2)).sqrt();
            assert!(distance(head)<distance(tip),"麦头应在嘴边，麦柄朝外");
            assert!(head[0]-2.6>1.4 && head[1]-2.6>17.0,"不能遮住嘴或眼睛");
            assert!(tip[0]+1.0<16.0 && tip[1]+1.0<28.0,"摆动不能超出常驻槽位");
            heights.push(head[1]);angles.push(angle);
        }
        let range=|values:Vec<f32>|values.iter().copied().fold(f32::NEG_INFINITY,f32::max)-values.iter().copied().fold(f32::INFINITY,f32::min);
        assert!((2.0..2.4).contains(&range(heights)), "换气和唱歌时的麦头要看得出高度变化");
        assert!((0.39..0.64).contains(&range(angles)), "麦柄随重拍转动");
    }

    #[test]
    fn all_singing_poses_visibly_move_throughout_the_hold_at_native_size() {
        use eframe::egui::{self, Pos2, Rect, Vec2};
        if !crate::bar_layout::animations_enabled() { return; }
        let ctx = egui::Context::default();
        let mut raster = crate::preview_raster::Raster::new(48, 32);
        let mut segment_start = 0.0;
        for expression in MUSIC_EXPRESSIONS.into_iter().take(6) {
            let mut pet = Pet { music_age: segment_start + 1.0, ..Default::default() };
            let pose = pet.music_pose();
            let mut previous = Vec::new();
            let mut quarter = Vec::new();
            let mut moving = 0;
            let mut strongest = 0;
            for frame in 0..(14 * 60) {
                pet.step(PetState::Music, frame as f64 / 60.0, 1.0 / 60.0, [0.0; 2], true);
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(48.0, 32.0))),
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        pet.draw(&Card::new(ui, Pos2::ZERO, ui.max_rect()), 24.0, 0.0, None);
                    });
                });
                raster.update_textures(&output.textures_delta);
                raster.buf.fill([0.0; 4]);
                raster.draw(&ctx.tessellate(output.shapes, 1.0), 1.0);
                let pixels: Vec<u8> = raster.buf.iter().map(|p| (p[0] * 255.0).round() as u8).collect();
                let changed = |old: &[u8], contrast: u8| old.iter().zip(&pixels)
                    .filter(|(a,b)| a.abs_diff(**b) >= contrast).count();
                if changed(&previous, 2) >= 6 { moving += 1; }
                if frame % 15 == 0 {
                    strongest = strongest.max(changed(&quarter, 64));
                    quarter.clone_from(&pixels);
                }
                if frame % 60 == 59 {
                    assert!(moving >= 56 && strongest >= 28,
                        "{pose:?}, second {}: {moving} moving frames, {strongest} visibly displaced pixels", frame / 60);
                    moving = 0;
                    strongest = 0;
                }
                previous = pixels;
                raster.free_textures(&output.textures_delta);
            }
            segment_start += MUSIC_PERIOD + expression.duration();
        }
    }
    #[test]
    #[ignore="按需导出听歌状态的实际界面动图"]
    fn listening_expression_gallery(){
        use eframe::egui::{self,Pos2,Rect,Vec2};
        use crate::app::{App,Env,Options,WIN_W,WIN_H};
        let dest=std::path::PathBuf::from(std::env::var_os("DAOZAI_LISTENING_PREVIEW").expect("需指定预览目录"));
        std::fs::create_dir_all(&dest).unwrap();
        std::env::set_var("DAOZAI_DATA_DIR",std::env::temp_dir().join(format!("daozai-listening-{}",std::process::id())));
        let ctx=egui::Context::default();let mut app=App::new(&ctx,Options{fetch_lyrics:false,pet_idle:1e9});
        app.settings.pet=true;app.settings.minimal=false;app.settings.island_lyrics=false;
        let mut scenes=Vec::new();let mut start=0.0;
        for (index,expression) in MUSIC_EXPRESSIONS.into_iter().enumerate() {
            if [0,2].contains(&index) {scenes.push((format!("演唱动作 {}",index+1),start));}
            if [0,1,3,4,5,11].contains(&index) {
                let name=match expression {Expression::Wink=>"调皮眨眼",Expression::Delighted=>"偷偷开心",Expression::Blush=>"害羞脸红",
                    Expression::Heart=>"悄悄比心",Expression::Scan=>"慢慢巡视",_=>"左右张望"};
                scenes.push((name.to_owned(),start+MUSIC_PERIOD));
            }
            start+=MUSIC_PERIOD+expression.duration();
        }
        let (fps, frames) = if std::env::var_os("DAOZAI_SINGING_PHRASE").is_some() {
            scenes.truncate(1);
            scenes[0].1 += 2.0;
            (60.0, 308)
        } else { (10.0, 32) };
        let mut raster=crate::preview_raster::Raster::new(1280,540);let mut sequence=0;
        for (index,(_,at)) in scenes.iter().enumerate() {
            app.pet.music_age=*at;
            for frame in 0..frames {
                let t=sequence as f64/fps;sequence+=1;
                let env=Env{t,clock:"09:41:00".into(),today:"2026-11-07".into(),date:"11月7日 周六".into(),idle_seconds:Some(0.0),
                    media:crate::smtc::NowPlaying{title:"晚风与星光".into(),artist:"示例歌手".into(),playing:true,duration:240.0,..Default::default()},..Default::default()};
                let input=egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(WIN_W,WIN_H))),time:Some(t),
                    viewports:std::iter::once((egui::ViewportId::ROOT,egui::ViewportInfo{native_pixels_per_point:Some(2.0),..Default::default()})).collect(),..Default::default()};
                let output=ctx.run(input,|ctx|{egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx,|ui|{app.frame(ui,ui.max_rect(),&env);});});
                raster.update_textures(&output.textures_delta);raster.buf.fill([0.95,0.96,0.98,1.0]);
                raster.draw(&ctx.tessellate(output.shapes,output.pixels_per_point),output.pixels_per_point);
                let mut rgba=Vec::new();raster.to_rgba8(&mut rgba);let image=image::RgbaImage::from_raw(1280,540,rgba).unwrap();
                image::imageops::crop_imm(&image,420,0,440,100).to_image().save(dest.join(format!("music-{index:02}-{frame:03}.png"))).unwrap();
                raster.free_textures(&output.textures_delta);
            }
        }
        std::fs::write(dest.join("scenes.json"),serde_json::to_string(&scenes).unwrap()).unwrap();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MusicPose { Microphone, Hum, SideSing, Squint, HighNote, Smile }

const MUSIC_PERIOD:f32=16.0;
const MUSIC_EXPRESSIONS:[Expression;12]=[
    Expression::Wink,Expression::Delighted,Expression::Smug,Expression::Blush,
    Expression::Heart,Expression::Scan,Expression::SideGlance,Expression::Giggle,
    Expression::Praised,Expression::Peek,Expression::ShyPeek,Expression::Search,
];
struct MusicSegment {
    pose:MusicPose,
    elapsed:f32,
    expression:Option<(Expression,f32)>,
}
fn music_segment_at(age:f32)->MusicSegment {
    let cycle=MUSIC_EXPRESSIONS.iter().map(|expression|MUSIC_PERIOD+expression.duration()).sum::<f32>();
    let mut time=age%cycle;
    for (index,expression) in MUSIC_EXPRESSIONS.into_iter().enumerate() {
        let pose=match index%6 {0=>MusicPose::Microphone,1=>MusicPose::Hum,2=>MusicPose::SideSing,3=>MusicPose::Squint,4=>MusicPose::HighNote,_=>MusicPose::Smile};
        if time<MUSIC_PERIOD {return MusicSegment{pose,elapsed:time,expression:None};}
        time-=MUSIC_PERIOD;
        if time<expression.duration(){return MusicSegment{pose,elapsed:MUSIC_PERIOD,expression:Some((expression,time))};}
        time-=expression.duration();
    }
    MusicSegment{pose:MusicPose::Microphone,elapsed:0.0,expression:None}
}

// 一口气唱四拍。嘴形在每拍内改变，头随重拍摆动，麦克风在换气时略微放低。
#[derive(Clone, Copy)]
struct SingingMotion {
    beat: f32,
    sway: f32,
    vowel: f32,
    breath: f32,
}

impl SingingMotion {
    fn at(elapsed: f32, animate: bool) -> Self {
        if !animate { return Self { beat: 0.0, sway: 0.0, vowel: 0.45, breath: 0.0 }; }
        let beats = elapsed / 0.64;
        let phase = beats * std::f32::consts::TAU;
        let breath = gesture(beats % 4.0, 3.12, 3.65, 4.0);
        let syllable = 0.5 - 0.5 * (phase + 0.65 * (phase * 0.5).sin()).cos();
        Self {
            beat: phase.sin(), sway: (phase * 0.25).sin(),
            vowel: syllable * (1.0 - breath), breath,
        }
    }

    fn microphone(self) -> ([f32; 2], f32) {
        let head = [5.7 + 1.15 * self.breath - 0.45 * self.vowel,
                    20.7 + 1.6 * self.breath - 0.65 * self.vowel];
        let angle = 0.46 + 0.20 * self.beat + 0.17 * self.breath;
        (head, angle)
    }
}

fn gesture(age: f32, start: f32, peak: f32, end: f32) -> f32 {
    smoothstep(start, peak, age) * (1.0 - smoothstep(peak, end, age))
}

fn microphone(c:&Card,cx:f32,y:f32,movement:SingingMotion){
    let (head,angle)=movement.microphone();
    let point=|s:f32,v:f32|(cx+head[0]+s*angle.cos()-v*angle.sin(),y+head[1]+s*angle.sin()+v*angle.cos());
    let capsule=|start:f32,end:f32,width:f32,color:Color32|{
        let a=point(start,0.0);let b=point(end,0.0);
        c.seg(a,b,width,color);c.circle(a.0,a.1,width/2.0,color);c.circle(b.0,b.1,width/2.0,color);
    };
    let line=|a:(f32,f32),b:(f32,f32),width:f32,color:Color32|c.seg(point(a.0,a.1),point(b.0,b.1),width,color);
    capsule(2.3,7.0,1.9,Color32::from_rgb(176,183,197));
    capsule(2.5,6.8,1.45,Color32::from_rgb(76,84,101));
    line((3.4,-0.53),(6.4,-0.53),0.28,Color32::from_rgb(218,225,237));
    capsule(2.05,2.65,2.6,Color32::from_rgb(126,137,157));
    let head=point(0.0,0.0);
    c.circle(head.0,head.1,2.6,Color32::from_rgb(222,228,238));
    c.circle(head.0,head.1,2.15,Color32::from_rgb(164,176,194));
    for s in [-1.0f32,0.0,1.0] {let v=(4.0-s*s).sqrt()*0.88;line((s,-v),(s,v),0.32,Color32::from_rgb(92,105,126));}
    for v in [-1.0f32,0.0,1.0] {let s=(4.0-v*v).sqrt()*0.88;line((-s,v),(s,v),0.3,Color32::from_rgb(226,232,243));}
}

#[derive(Clone, Copy)]
struct Eye {
    height: f32,
    spread: f32,
    curve: f32,
    y: f32,
    tilt: f32,
}

impl Eye {
    fn open(height: f32) -> Self { Self { height, spread: 0.0, curve: 0.0, y: 0.0, tilt: 0.0 } }
    fn arc(spread: f32, curve: f32) -> Self { Self { spread, curve, ..Self::open(0.0) } }
    fn draw_straight(&self, c: &Card, x: f32, y: f32, radius: f32) {
        use eframe::egui::vec2;
        let axis = vec2(self.spread, self.height * 0.5 + self.tilt);
        if axis.length() < 0.01 {
            c.circle(x, y, radius, theme::WHITE);
            return;
        }
        let along = axis.normalized();
        let side = vec2(along.y, -along.x);
        // 描边和端点若分别抗锯齿，交叠处会更亮。整只眼睛只填充一个轮廓。
        let mut outline = [(0.0, 0.0); 18];
        for (i, point) in outline.iter_mut().enumerate() {
            let sign = if i < 9 { -1.0 } else { 1.0 };
            let angle = std::f32::consts::PI * (i % 9) as f32 / 8.0;
            let (sin, cos) = angle.sin_cos();
            let p = (axis + (side * cos + along * sin) * radius) * sign;
            *point = (x + p.x, y + p.y);
        }
        c.poly(&outline, theme::WHITE);
    }
    fn follow(&mut self, goal: Self, dt: f32) {
        self.height = approach(self.height, goal.height, 17.0, dt);
        self.spread = approach(self.spread, goal.spread, 17.0, dt);
        self.curve = approach(self.curve, goal.curve, 17.0, dt);
        self.y = approach(self.y, goal.y, 12.0, dt);
        self.tilt = approach(self.tilt, goal.tilt, 12.0, dt);
    }
}

pub(crate) struct Pet {
    pub state: PetState,
    pub reaction: Option<(PetState, f32)>,
    pub waking_until: f64,
    pub last_idle: f64,
    pub hovered: bool,
    pub worried: bool,
    pub expressions: crate::pet_expressions::Expressions,
    age: f32,
    music_age:f32,
    eyes: [Eye; 2],
    gaze: [f32; 2],
    y: f32,
}

impl Default for Pet {
    fn default() -> Self {
        Self { state: PetState::Awake, reaction: None, waking_until: 0.0, last_idle: 0.0,
            hovered: false, worried: false, expressions:Default::default(), age: 0.0, music_age:0.0, eyes: [Eye::open(4.4); 2], gaze: [0.0; 2], y: 0.0 }
    }
}

impl Pet {
    pub fn react(&mut self, state: PetState) { self.reaction = Some((state, self.expressions.react(state))); self.age = 0.0; }
    /// 反馈只在实际可见时计时，避免用户还在面板里操作时就播完。
    pub fn advance(&mut self, dt: f32, visible: bool) {
        if visible {
            if let Some((_, remaining)) = &mut self.reaction {
                *remaining -= dt;
                if *remaining <= 0.0 { self.reaction = None; }
            }
        }
    }
    #[cfg(test)]
    fn music_pose(&self)->MusicPose {
        music_segment_at(self.music_age).pose
    }

    pub fn step(&mut self, state: PetState, t: f64, dt: f32, gaze: [f32; 2], visible: bool) {
        if self.state==PetState::Music && matches!(state,PetState::Awake|PetState::Curious) {self.expressions.clear();}
        self.expressions.step(state,dt,visible);
        if state != self.state { self.state = state; self.age = 0.0; }
        if !visible { return; }
        self.age += dt;
        if state==PetState::Music {
            self.music_age+=dt;
            if let Some((expression,time))=music_segment_at(self.music_age).expression {self.expressions.play_at(expression,time);}
        }
        self.follow_pose(t, dt, gaze, crate::bar_layout::animations_enabled());
    }

    fn follow_pose(&mut self, t: f64, dt: f32, gaze: [f32; 2], motion: bool) {
        let state = self.state;
        let age = self.age;
        let mut eyes = [Eye::open(4.4); 2];
        let breath = if motion { (age * 1.9).sin() } else { 0.0 };
        let mut y = breath * 0.8;
        let mut look = gaze;
        let mut eye_dt = dt;
        match state {
            PetState::Awake if motion => {
                let phase = age % 24.0;
                let lean = gesture(phase, 3.0, 3.8, 5.1) - gesture(phase, 15.0, 15.8, 17.1);
                eyes[0].y = lean * 1.25;
                eyes[1].y = -eyes[0].y;
                for eye in &mut eyes { eye.spread = lean * 0.4; eye.height += breath * 0.5; }
            },
            PetState::Awake => {},
            PetState::Curious => {
                eyes[0].height = 5.6 + breath * 0.6; eyes[1].height = 6.3 - breath * 0.6;
                eyes[0].y = 0.6 + breath * 0.55; eyes[1].y = -eyes[0].y;
            },
            PetState::Waking => {
                let wake = if motion { age } else { 2.2 };
                let stretch = gesture(wake, 0.55, 1.18, 1.95);
                eyes[0].height = lerp(0.0, 4.4, smoothstep(0.1, 0.65, wake)) + stretch * 1.8;
                eyes[1].height = lerp(0.0, 4.4, smoothstep(0.4, 1.1, wake)) + stretch * 1.8;
                y = 1.0 - smoothstep(0.0, 1.2, wake) - stretch * 1.5;
                look = [0.0; 2];
            },
            PetState::Happy => {
                eyes = [Eye::arc(2.6, -1.8); 2];
                y = if motion { -(age * 7.5).sin().abs() * 2.0 * (-age * 0.65).exp() } else { 0.0 };
            },
            PetState::Satisfied => {
                eyes = [Eye::arc(2.2, -1.2); 2];
                y = if motion { (age * 8.0).sin() * 1.2 * (-age * 0.8).exp() } else { 0.0 };
            },
            PetState::Music => {
                let segment = music_segment_at(self.music_age);
                let m = SingingMotion::at(segment.elapsed, motion);
                eye_dt *= lerp(0.2, 1.0, smoothstep(0.0, 1.0, segment.elapsed));
                look = [m.sway * 1.55, 0.0];
                y = -0.45 + m.beat * 1.35 + m.breath * 0.55;
                let mut tilt = m.sway * 0.75;
                let eye = match segment.pose {
                    MusicPose::Microphone => Eye::arc(2.5 - 0.45 * m.vowel, 0.5 + 1.45 * m.vowel),
                    MusicPose::SideSing => {
                        look[0] = 0.6 + m.sway;
                        tilt += 0.65;
                        Eye::open(1.0 + m.vowel * 1.9)
                    },
                    MusicPose::HighNote => {
                        y -= 1.25 * m.vowel;
                        Eye::arc(2.5, -0.45 - m.vowel * 1.55)
                    },
                    MusicPose::Hum => Eye::arc(2.65, 0.3 + m.vowel),
                    MusicPose::Squint => Eye::arc(1.55 + m.vowel * 0.85, m.vowel * 0.55),
                    MusicPose::Smile => {
                        tilt += 0.4;
                        Eye::arc(2.45, -0.5 - m.vowel * 1.3)
                    },
                };
                eyes = [eye; 2];
                for (eye, sign) in eyes.iter_mut().zip([-1.0, 1.0]) {
                    eye.y = tilt * sign;
                    eye.tilt = tilt * 0.36;
                }
            },
            PetState::Worried => {
                eyes[0] = Eye { spread: -0.6, tilt: -0.7, y: 0.4, ..Eye::open(2.8) };
                eyes[1] = Eye { spread: 0.6, tilt: 0.7, y: 1.0, ..Eye::open(1.8) };
                eyes[1].height += breath * 0.8;
            },
            PetState::Focused => {
                let phase = age % 17.0;
                let nod = if motion { gesture(phase, 4.0, 4.55, 5.5) } else { 0.0 };
                eyes = [Eye::open(1.8 - nod * 0.9); 2];
                look = [gaze[0] * 0.2, 0.0];
                y = nod * 1.7 + breath * 0.7;
                look[0] += if motion { (age * 1.1).sin() * 1.3 } else { 0.0 };
            },
            PetState::Dozing => {
                let phase = age % 9.5;
                let droop = if motion { gesture(phase, 1.0, 3.1, 3.9) } else { 0.0 };
                eyes = [Eye::open(0.6 * (1.0 - droop)); 2];
                eyes[1].y = droop * 0.8;
                y = 1.0 + droop * 1.4 + breath * 0.65;
                look = [0.0; 2];
            },
            PetState::Sleeping => {
                eyes = [Eye::arc(2.4, 1.0); 2];
                y = 0.8 + if motion { (age * 1.2).sin() * 1.05 } else { 0.0 };
                look = [0.0; 2];
            },
        }
        if self.expressions.current().is_some() {eyes=[Eye::open(4.4);2];look=[0.0;2];y=0.0;}
        if motion && self.expressions.current().is_none() && matches!(state, PetState::Awake | PetState::Curious | PetState::Focused | PetState::Worried) {
            let start = 2.0 + crate::anim::hash01((t / 5.6).floor() as u64) * 2.7;
            let blink = ((t % 5.6) as f32 - start) / 0.18;
            if (0.0..1.0).contains(&blink) {
                for eye in &mut eyes { eye.height *= 1.0 - (blink * std::f32::consts::PI).sin(); }
            }
        }
        for (eye, goal) in self.eyes.iter_mut().zip(eyes) { eye.follow(goal, eye_dt); }
        for (axis, goal) in self.gaze.iter_mut().zip(look) { *axis = approach(*axis, goal, 10.0, dt); }
        self.y = approach(self.y, y, 14.0, dt);
    }

    pub fn draw(&mut self, c: &Card, cx: f32, _t: f64, _palette:Option<crate::skin_paint::Palette>) {
        if self.expressions.draw(c,cx) {return;}
        for (i, eye) in self.eyes.iter().enumerate() {
            let x = cx + if i == 0 { -5.6 } else { 5.6 } + self.gaze[0];
            let y = 14.5 + self.y + eye.y + self.gaze[1];
            let stroke=if self.state==PetState::Music {3.6-0.8*smoothstep(0.5,1.5,eye.spread.abs())}else{3.6};
            if eye.curve.abs() < 0.01 {
                eye.draw_straight(c, x, y, stroke / 2.0);
                continue;
            }
            let points: Vec<_> = (0..=12).map(|n| {
                let q = n as f32 / 6.0 - 1.0;
                (x + q * eye.spread, y + q * (eye.height / 2.0 + eye.tilt) + (1.0 - q*q) * eye.curve)
            }).collect();
            c.path(&points, stroke, theme::WHITE);
            // 圆端点保持胶囊眼睛的轮廓。
            for p in [points[0], points[12]] { c.circle(p.0, p.1, stroke/2.0, theme::WHITE); }
        }
        match self.state {
            PetState::Dozing | PetState::Sleeping => {
                let bob = if crate::bar_layout::animations_enabled() { ((self.age * 1.4).sin() + 1.0) * 0.7 } else { 0.7 };
                c.text(cx+14.0, 13.0-bob, "z", 7.0, W::Bold, Color32::from_rgb(170,160,255), H::Left);
                if self.state == PetState::Sleeping { c.text(cx+19.0, 9.0-bob, "z", 5.5, W::Bold, Color32::from_rgb(170,160,255), H::Left); }
            },
            PetState::Happy => {
                let k = smoothstep(0.0, 0.2, self.age) * (1.0-smoothstep(1.5, 2.5, self.age));
                let c = c.faded(k);
                for (dx, y, r) in [(-18.0, 9.0, 2.1), (18.0, 8.0, 2.8)] {
                    c.seg((cx+dx-r,y),(cx+dx+r,y),1.0,theme::GREEN);
                    c.seg((cx+dx,y-r),(cx+dx,y+r),1.0,theme::GREEN);
                }
            },
            PetState::Satisfied => crate::paint::droplet(c,cx+18.0,12.0,5.0,theme::CYAN),
            PetState::Music => {
                let segment=music_segment_at(self.music_age);
                let movement = SingingMotion::at(segment.elapsed, crate::bar_layout::animations_enabled());
                self.draw_music(&c.faded(smoothstep(0.0,1.0,segment.elapsed)), cx, segment.pose, movement);
            },
            PetState::Worried => {
                c.fill(cx+17.0,8.0,1.6,4.2,0.8,theme::YELLOW);
                c.circle(cx+17.8,14.5,0.9,theme::YELLOW);
            },
            _ => {},
        }
    }

    fn draw_music(&self, c: &Card, cx: f32, pose: MusicPose, m: SingingMotion) {
        let face_x = cx + self.gaze[0];
        let y = self.y + self.gaze[1];
        let mouth_x = face_x - 1.15;
        let mouth_y = 20.3 + y + 0.6 * m.vowel;
        let mouth = |rx: f32, ry: f32| {
            let points = std::array::from_fn::<_, 25, _>(|i| {
                let (sin, cos) = (i as f32 / 24.0 * std::f32::consts::TAU).sin_cos();
                (mouth_x + cos * rx, mouth_y + sin * ry)
            });
            c.path(&points, 1.0, theme::WHITE);
        };
        match pose {
            MusicPose::Microphone => mouth(2.05 - 0.75 * m.vowel, 0.23 + 1.7 * m.vowel),
            MusicPose::SideSing => mouth(2.15 - 0.55 * m.vowel, 0.3 + 1.55 * m.vowel),
            MusicPose::HighNote => mouth(1.6 + 0.35 * m.vowel, 0.9 + 1.5 * m.vowel),
            MusicPose::Squint => mouth(1.75 - 0.45 * m.vowel, 0.23 + 1.65 * m.vowel),
            MusicPose::Hum | MusicPose::Smile => {
                let width = 2.25 + 0.4 * m.vowel;
                let depth = 0.55 + 1.3 * m.vowel;
                let center = if pose == MusicPose::Hum { depth * 0.25 } else { depth * 1.25 };
                c.path(&[(mouth_x-width, mouth_y), (mouth_x-width*0.5, mouth_y+depth),
                         (mouth_x, mouth_y+center), (mouth_x+width*0.5, mouth_y+depth),
                         (mouth_x+width, mouth_y)], 1.0, theme::WHITE);
            },
        }
        if matches!(pose,MusicPose::Microphone|MusicPose::SideSing|MusicPose::HighNote) {
            microphone(c, face_x, y, m);
        } else {
            let x = cx + 13.2 + 1.0 * m.sway;
            let y = 8.5 - 1.85 * m.beat;
            let tilt = m.sway * 0.95;
            c.seg((x+tilt, y-2.8), (x, y+2.8), 1.1, theme::WHITE);
            c.seg((x+tilt, y-2.8), (x+tilt+2.4, y-1.8), 1.1, theme::WHITE);
            c.circle(x-1.15, y+2.8, 1.45, theme::WHITE);
        }
    }
}
