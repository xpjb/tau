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
        hits
    }
    pub(super) fn test_projects(&self)->Vec<(Rect,String)>{self.test_hits().into_iter().filter_map(|h| if let Action::SelectProject(id)=h.action {Some((h.rect,id))}else{None}).collect()}
    pub(super) fn test_chats(&self)->Vec<(Rect,String)>{let mut rows=self.test_hits().into_iter().filter_map(|h| if let Action::Select(id)=h.action {Some((h.rect,id))}else{None}).collect::<Vec<_>>();rows.extend(self.root.legacy.chat_areas.clone());rows}
}
