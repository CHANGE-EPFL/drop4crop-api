from sqlmodel import SQLModel, Field, UniqueConstraint, Relationship
from uuid import uuid4, UUID
from sqlalchemy.sql import func
import datetime
import enum
from typing import Any
from app.layers.links import LayerCountryLink
from app.styles.models import Style


class LayerVariables(str, enum.Enum):
    crop = "crop"
    water_model = "water_model"
    climate_model = "climate_model"
    scenario = "scenario"
    variable = "variable"
    year = "year"


class LayerGroupsRead(SQLModel):
    crop: list[str] = []
    water_model: list[str | None] = []
    climate_model: list[str | None] = []
    scenario: list[str | None] = []
    variable: list[str] = []
    year: list[int | None] = []


class LayerBase(SQLModel):
    layer_name: str | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    crop: str | None = Field(
        default=None,
        index=True,
        nullable=True,
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
        nullable=True,
    )
    year: int | None = Field(
        default=None,
        index=True,
        nullable=True,
    )
    enabled: bool = Field(
        default=False,
        nullable=False,
    )
    style_id: UUID | None = Field(
        foreign_key="style.id",
        default=None,
        nullable=True,
    )
    global_average: float | None = Field(
        default=None,
        index=True,
        nullable=True,
    )

    filename: str | None = Field(default=None)

    min_value: float | None = Field(default=None)
    max_value: float | None = Field(default=None)

    uploaded_at: datetime.datetime = Field(
        default_factory=datetime.datetime.now,
        title="Uploaded At",
        description="Date and time when the record was uploaded",
        sa_column_kwargs={
            "default": func.now(),
        },
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

    class Config:
        arbitrary_types_allowed = True


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

    country_values: list[LayerCountryLink] = Relationship(
        back_populates="layer",
        sa_relationship_kwargs={
            "lazy": "selectin",
            "cascade": "all,delete,delete-orphan",
        },
    )

    style: Style = Relationship(
        back_populates="layers",
        sa_relationship_kwargs={
            "lazy": "selectin",
        },
    )


class CountrySimple(SQLModel):
    """Without geometry"""

    id: UUID
    name: str
    iso_a2: str
    iso_a3: str
    iso_n3: int


class CountryValue(SQLModel):
    var_wf: float | None = None
    var_wfb: float | None = None
    var_wfg: float | None = None
    var_vwc: float | None = None
    var_vwcb: float | None = None
    var_vwcg: float | None = None
    var_wdb: float | None = None
    var_wdg: float | None = None

    country: CountrySimple | None = None


class LayerRead(SQLModel):
    layer_name: str | None
    global_average: float | None
    country_values: list[CountryValue] = []
    style: Any | None = None


class LayerReadAuthenticated(LayerBase):
    id: UUID
    enabled: bool
    created_at: str | None = None
    style: Any | None = None


class LayerCreate(LayerBase):
    pass


class LayerUpdate(LayerBase):
    style_name: str | None = None


class LayerUpdateBatch(SQLModel):
    ids: list[UUID]
    data: Any
