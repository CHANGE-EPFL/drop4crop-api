from sqlmodel import SQLModel


class StyleCreate(SQLModel):
    name: str
    sld: str
