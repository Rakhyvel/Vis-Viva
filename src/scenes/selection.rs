use hecs::Entity;

#[derive(Debug)]
pub enum SelectionKind {
    Craft,
    Body,
    Building,
}

pub struct SelectionState {
    pub crafts: Vec<Entity>,
    pub bodies: Vec<Entity>,
    pub buildings: Vec<Entity>,

    pub selected: Option<usize>,
    pub kind: SelectionKind,

    // For swoosh animation
    pub changed_at: f64,
}

impl SelectionState {
    pub fn new(crafts: Vec<Entity>, bodies: Vec<Entity>, buildings: Vec<Entity>) -> Self {
        Self {
            crafts,
            bodies,
            buildings,
            selected: None,
            kind: SelectionKind::Body,
            changed_at: 0.0,
        }
    }

    pub fn selected_entity(&self) -> Option<Entity> {
        self.selected.map(|s| self.curr_sel_track()[s])
    }

    pub fn set_selected(&mut self, entity: Entity, app_seconds: f64) {
        if let Some(selected) = self.selected_entity() {
            if selected == entity {
                return;
            }
        }

        let found = self
            .crafts
            .iter()
            .position(|e| *e == entity)
            .map(|x| (x, SelectionKind::Craft))
            .or(self
                .bodies
                .iter()
                .position(|e| *e == entity)
                .map(|x| (x, SelectionKind::Body)))
            .or(self
                .buildings
                .iter()
                .position(|e| *e == entity)
                .map(|x| (x, SelectionKind::Building)));

        if let Some((idx, kind)) = found {
            self.selected = Some(idx);
            self.kind = kind;

            self.changed_at = app_seconds;
        }
    }

    pub fn prev(&mut self, app_seconds: f64) {
        if let Some(selected) = self.selected {
            let mut new_selection = selected;
            if selected == 0 {
                new_selection = self.curr_sel_track().len() - 1;
            } else {
                new_selection -= 1;
            }
            self.selected = Some(new_selection);
        } else {
            self.selected = Some(0);
        }

        self.changed_at = app_seconds;
    }

    pub fn next(&mut self, app_seconds: f64) {
        if let Some(selected) = self.selected {
            let mut new_selection = selected + 1;
            if new_selection >= self.curr_sel_track().len() {
                new_selection = 0;
            }
            self.selected = Some(new_selection);
        } else {
            self.selected = Some(0);
        }

        self.changed_at = app_seconds;
    }

    fn curr_sel_track(&self) -> &Vec<Entity> {
        match self.kind {
            SelectionKind::Body => &self.bodies,
            SelectionKind::Craft => &self.crafts,
            SelectionKind::Building => &self.buildings,
        }
    }
}
