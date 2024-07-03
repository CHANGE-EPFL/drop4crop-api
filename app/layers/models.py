from sqlmodel import SQLModel, Field, UniqueConstraint
from uuid import uuid4, UUID
from sqlalchemy.sql import func
import datetime


class LayerBase(SQLModel):
    layer_name: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    crop: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    water_model: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    climate_model: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    scenario: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    variable: str = Field(
        default=None,
        index=True,
        nullable=False,
    )
    year: int = Field(
        default=None,
        index=True,
        nullable=False,
    )

    last_updated: datetime.datetime = Field(
        default_factory=datetime.datetime.now,
        title="Last Updated",
        description="Date and time when the record was last updated",
        sa_column_kwargs={
            "onupdate": func.now(),
            "server_default": func.now(),
        },
    )


class Layer(LayerBase, table=True):
    __table_args__ = (UniqueConstraint("id"),)
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


class LayerRead(LayerBase):
    id: UUID


class LayerCreate(LayerBase):
    pass


class LayerUpdate(LayerBase):
    pass
