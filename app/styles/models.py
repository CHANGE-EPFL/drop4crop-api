from sqlmodel import (
    SQLModel,
    Field,
    UniqueConstraint,
    Column,
    JSON,
    Relationship,
)
from uuid import uuid4, UUID
from sqlalchemy.sql import func
import datetime
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING:
    from app.layers.models import Layer


class StyleBase(SQLModel):
    name: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )

    style: list[Any] = Field(default=[], sa_column=Column(JSON))

    last_updated: datetime.datetime = Field(
        default_factory=datetime.datetime.now,
        title="Last Updated",
        description="Date and time when the record was last updated",
        sa_column_kwargs={
            "onupdate": func.now(),
            "server_default": func.now(),
        },
    )


class Style(StyleBase, table=True):
    __table_args__ = (
        UniqueConstraint("id"),
        UniqueConstraint("name"),
    )
    iterator: int = Field(
        default=None,
        nullable=False,
        primary_key=True,
        index=True,
    )
    id: UUID = Field(
        default_factory=uuid4,
        index=True,
        nullable=False,
    )

    layers: list["Layer"] = Relationship(
        back_populates="style",
        sa_relationship_kwargs={
            "lazy": "selectin",
        },
    )


class StyleRead(StyleBase):
    id: UUID


class StyleCreate(StyleBase):
    pass


class StyleUpdate(StyleBase):
    style_name: str | None = None
