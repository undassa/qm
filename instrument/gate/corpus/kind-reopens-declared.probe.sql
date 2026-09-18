UPDATE kind_layout SET spec = spec - 'reopens' WHERE name = 'task' AND $1 <> ''
