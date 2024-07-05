from sqlmodel import SQLModel, Field, UniqueConstraint
from uuid import uuid4, UUID
from sqlalchemy.sql import func
import datetime
from pydantic import constr
import enum


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
        nullable=False,
    )
    climate_model: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    scenario: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    variable: str | None = Field(
        default=None,
        index=True,
        nullable=False,
    )
    year: int | None = Field(
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


class LayerRead(SQLModel):
    layer_name: str


class LayerCreate(LayerBase):
    pass


class LayerUpdate(LayerBase):
    pass
