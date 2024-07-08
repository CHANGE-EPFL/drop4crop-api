from sqlmodel import SQLModel, Field, UniqueConstraint
from uuid import uuid4, UUID
from sqlalchemy.sql import func
import datetime
from pydantic import constr
import enum
from typing import Any


class LayerVariables(str, enum.Enum):
    crop = "crop"
    water_model = "water_model"
    climate_model = "climate_model"
    scenario = "scenario"
    variable = "variable"
    year = "year"


class LayerGroupsRead(SQLModel):
    crop: list[str] = []
    water_model: list[str] = []
    climate_model: list[str] = []
    scenario: list[str] = []
    variable: list[str] = []
    year: list[int] = []


class LayerBase(SQLModel):
    layer_name: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    crop: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    water_model: str | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    climate_model: str | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    scenario: str | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    variable: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    year: int | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    enabled: bool = Field(
        default=True,
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
    __table_args__ = (
        UniqueConstraint("id"),
        UniqueConstraint(
            "crop",
            "year",
            "variable",
            "scenario",
            "climate_model",
            "water_model",
        ),
        UniqueConstraint("layer_name"),
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


class LayerRead(LayerBase):
    id: UUID
    created_at: datetime.datetime | None = None
    style_name: str | None = None


class LayerCreate(LayerBase):
    pass


class LayerUpdate(LayerBase):
    style_name: str | None = None
