//! Test-only semantic selectors. These inspect placed widget controls; they are
//! never a runtime hit table or an input route.
use super::*;
impl App {
    pub(super) fn test_hits(&self)->Vec<Hit>{
        let mut hits=self.root.legacy.hits.iter().map(|h|Hit {rect:h.rect,action:h.action.clone()}).collect::<Vec<_>>();
        for (_,button,choice) in &self.root.sidebar.controls.items {
            use ui::sidebar::Choice;
            if let Some(r)=button.control.rect && button.control.enabled {
                let rect=crate::render::intersect(r,button.control.clip);if rect.width<=0.||rect.height<=0.{continue;}
                let action=match choice {Choice::Select(id)=>Action::Select(id.clone()),Choice::New=>Action::New,Choice::Settings=>Action::Settings,Choice::Info(i)=>Action::Info(i.clone())};hits.push(Hit {rect,action});
            }
        }
        for (_,button,choice) in &self.root.sidebar.projects.controls.items {
            if let Some(r)=button.control.rect {let rect=crate::render::intersect(r,button.control.clip);if rect.width<=0.||rect.height<=0.{continue;}
                hits.push(Hit {rect,action:match choice {ui::sidebar::TopicChoice::Select(id)=>Action::SelectProject(id.clone()),ui::sidebar::TopicChoice::New=>Action::NewProject}});
            }
        }
        if let Some(rect)=self.root.notice.body.rect {hits.push(Hit {rect,action:self.controller.notice.as_ref().and_then(|n|n.download.clone()).map_or(Action::DismissNotice,Action::OpenDownloadNotice)});}
        if let Some(rect)=self.root.notice.close.rect {hits.push(Hit {rect,action:Action::DismissNotice});}
        if let Some(rect)=self.root.composer.field.control.rect {hits.push(Hit {rect,action:Action::Focus(None)});}
        for (_,button,choice) in &self.root.composer.controls.items {use ui::composer::Choice;
            if let Some(rect)=button.control.rect && button.control.enabled {let action=match choice {Choice::RetryCreate=>Action::RetryCreate,Choice::Attach=>Action::Attach,Choice::Usage=>Action::Usage,Choice::Send=>Action::Send,Choice::Tail=>Action::Tail,Choice::Queue(op)=>Action::Queue(op.clone()),Choice::RemoveFile(id)=>Action::RemoveFile(id.clone()),Choice::Suggest(t)=>Action::Suggest(t.clone())};hits.push(Hit {rect,action});}
        }
        for (_,b,a) in &self.root.quick_models.controls.items {if let Some(r)=b.control.rect && b.control.enabled {let rect=crate::render::intersect(r,b.control.clip);if rect.height>0.{hits.push(Hit {rect,action:match a {ui::composer::ModelChoice::Select(s,m)=>Action::ChooseModel(s.clone(),m.clone()),ui::composer::ModelChoice::Configure=>Action::ModelSettings}});}}}
        if let Some(field)=self.root.code.view.as_ref().and_then(|v|v.search.as_ref()) && let Some(rect)=field.control.rect {hits.push(Hit {rect,action:Action::Focus(Some(code_view::SEARCH_FIELD))});}
        for (_,button,choice) in &self.root.code.controls.items {use code_view::Choice as C;
            if let Some(r)=button.control.rect && button.control.enabled {let rect=crate::render::intersect(r,button.control.clip);if rect.height<=0.||rect.width<=0.{continue;}let action=match choice {
                C::Files=>Action::Files,C::FileClose=>Action::FileClose,C::FileOpen(p,d)=>Action::FileOpen(p.clone(),*d),C::FileUp=>Action::FileUp,C::FileFindHere=>Action::FileFindHere,C::FileFind=>Action::FileFind,C::FileClear=>Action::FileClear,C::FileCopy=>Action::FileCopy,C::FilePage(next)=>Action::FilePage(*next)};hits.push(Hit {rect,action});}
        }
        hits
    }
    pub(super) fn test_projects(&self)->Vec<(Rect,String)>{self.test_hits().into_iter().filter_map(|h| if let Action::SelectProject(id)=h.action {Some((h.rect,id))}else{None}).collect()}
    pub(super) fn test_chats(&self)->Vec<(Rect,String)>{let mut rows=self.test_hits().into_iter().filter_map(|h| if let Action::Select(id)=h.action {Some((h.rect,id))}else{None}).collect::<Vec<_>>();rows.extend(self.root.legacy.chat_areas.clone());rows}
}
